//! Sentence vectors for the memory index: BAAI's bge-small-en-v1.5 (MIT), a small
//! transformer trained to put a question near the passage that answers it. It ships with
//! the app at half precision (scripts/fetch-models.sh) and runs in this process on the CPU
//! with Apple's Accelerate framework, whatever brain is chosen. Nothing is downloaded at
//! run time and no text leaves the Mac. Calls are synchronous; run them on a blocking thread.
//!
//! Memory stays small: texts run in short batches of similar length, so little padding
//! is computed, and the model is freed after a few idle minutes and loaded again when needed.
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, MutexGuard, OnceLock, TryLockError,
    },
    time::{Duration, Instant},
};
use tokenizers::{Tokenizer, TruncationParams};

/// Names the vectors this model makes. Stored vectors from any other model are redone.
pub const MODEL_ID: &str = "bge-small-en-v1.5@5c38ec7";
pub const DIMENSION: usize = 384;
/// Index records are at most 1,500 characters, well inside this.
const MAX_TOKENS: usize = 512;
/// bge v1.5 matches short questions to passages better with this prefix on the question.
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
/// Texts run together at most.
const MAX_BATCH: usize = 8;
/// Texts × tokens² in one run. Each attention layer holds a few (texts, 12 heads, tokens,
/// tokens) f32 tensors at once; this keeps each near 25 MB, so a spike stays near 100 MB.
const ATTENTION_BUDGET: usize = 2 * MAX_TOKENS * MAX_TOKENS;
/// Unused this long, the model (about 130 MB) is freed.
const IDLE: Duration = Duration::from_secs(5 * 60);
/// What `embed_within` gives when indexing holds the model; search then uses keywords.
pub const BUSY: &str = "Memory search is busy indexing.";

struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    pad: u32,
}

struct Loaded {
    embedder: Embedder,
    last_used: Instant,
}

static DIR: OnceLock<PathBuf> = OnceLock::new();
static MODEL: Mutex<Option<Loaded>> = Mutex::new(None);
/// Whether a thread is watching for the model to go idle.
static WATCHING: AtomicBool = AtomicBool::new(false);

/// Where the bundled model lives; set once at startup.
pub fn set_model_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

fn load(dir: &Path) -> Result<Embedder, String> {
    let missing = || "Buntini's memory search model is missing from this installation.".to_string();
    let config: Config = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).map_err(|_| missing())?)
        .map_err(|e| format!("The memory search model's settings are invalid: {e}"))?;
    let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
        .map_err(|e| format!("The memory search tokenizer couldn't load: {e}"))?;
    // Each batch is padded to its own longest text below, not here.
    tokenizer.with_padding(None);
    tokenizer
        .with_truncation(Some(TruncationParams { max_length: MAX_TOKENS, ..Default::default() }))
        .map_err(|e| format!("The memory search tokenizer couldn't load: {e}"))?;
    let pad = tokenizer.token_to_id("[PAD]").unwrap_or(0);
    let weights = dir.join("model.safetensors");
    if !weights.is_file() {
        return Err(missing());
    }
    // SAFETY: the bundled file is read-only for the app's lifetime; the map is never written.
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights], DType::F32, &Device::Cpu) }
        .map_err(|e| format!("The memory search model couldn't load: {e}"))?;
    let model = BertModel::load(vb, &config).map_err(|e| format!("The memory search model couldn't load: {e}"))?;
    Ok(Embedder { model, tokenizer, pad })
}

/// A unit-length vector as one signed byte per dimension (value × 127).
pub fn quantize(vector: &[f32]) -> Option<Vec<i8>> {
    let length = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    (length.is_finite() && length > 0.0)
        .then(|| vector.iter().map(|v| (v / length * 127.0).round().clamp(-127.0, 127.0) as i8).collect())
}

/// One vector per text (None for blank text). `query` marks a search question rather
/// than a record being indexed. Waits while another call is using the model.
pub fn embed(texts: &[String], query: bool) -> Result<Vec<Option<Vec<i8>>>, String> {
    let model = MODEL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    run(model, texts, query)
}

/// Like `embed`, but gives up with `BUSY` when the model is still in use after `wait`, so a
/// question asked while indexing runs answers by keyword instead of waiting.
pub fn embed_within(texts: &[String], query: bool, wait: Duration) -> Result<Vec<Option<Vec<i8>>>, String> {
    let deadline = Instant::now() + wait;
    loop {
        match MODEL.try_lock() {
            Ok(model) => return run(model, texts, query),
            Err(TryLockError::Poisoned(poisoned)) => return run(poisoned.into_inner(), texts, query),
            Err(TryLockError::WouldBlock) if Instant::now() >= deadline => return Err(BUSY.into()),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Frees the model once it has gone unused for `idle`. True when nothing is loaded now.
fn release_if_idle(idle: Duration) -> bool {
    let mut model = match MODEL.try_lock() {
        Ok(model) => model,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return false,
    };
    if model.as_ref().is_some_and(|loaded| loaded.last_used.elapsed() < idle) {
        return false;
    }
    *model = None;
    WATCHING.store(false, Ordering::SeqCst);
    true
}

fn run(mut model: MutexGuard<'_, Option<Loaded>>, texts: &[String], query: bool) -> Result<Vec<Option<Vec<i8>>>, String> {
    let dir = DIR.get().ok_or("Buntini's memory search model isn't set up.")?;
    let kept: Vec<(usize, String)> = texts
        .iter()
        .enumerate()
        .filter(|(_, text)| !text.trim().is_empty())
        .map(|(index, text)| (index, if query { format!("{QUERY_PREFIX}{text}") } else { text.clone() }))
        .collect();
    let mut vectors = vec![None; texts.len()];
    if kept.is_empty() {
        return Ok(vectors);
    }
    if model.is_none() {
        *model = Some(Loaded { embedder: load(dir)?, last_used: Instant::now() });
        // The watcher checks under this same lock, so exactly one runs while loaded.
        if !WATCHING.swap(true, Ordering::SeqCst) {
            std::thread::spawn(|| loop {
                std::thread::sleep(Duration::from_secs(60));
                if release_if_idle(IDLE) {
                    return;
                }
            });
        }
    }
    let loaded = model.as_mut().expect("the model was loaded above");
    loaded.last_used = Instant::now();
    let embedder = &loaded.embedder;
    let failed = |e: &dyn std::fmt::Display| format!("Memory search couldn't read this text: {e}");
    let encodings = embedder
        .tokenizer
        .encode_batch(kept.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>(), true)
        .map_err(|e| failed(&e))?;
    // Shortest first, so each batch pads only to texts of about its own length.
    let mut order: Vec<usize> = (0..encodings.len()).collect();
    order.sort_by_key(|&i| encodings[i].get_ids().len());
    for group in batches(&order.iter().map(|&i| encodings[i].get_ids().len()).collect::<Vec<_>>()) {
        let members = &order[group];
        let longest = members.iter().map(|&i| encodings[i].get_ids().len()).max().unwrap_or(1).max(1);
        let mut ids = Vec::with_capacity(members.len() * longest);
        let mut mask = Vec::with_capacity(members.len() * longest);
        for &i in members {
            let row = encodings[i].get_ids();
            ids.extend_from_slice(row);
            ids.extend(std::iter::repeat_n(embedder.pad, longest - row.len()));
            mask.extend(std::iter::repeat_n(1u32, row.len()));
            mask.extend(std::iter::repeat_n(0u32, longest - row.len()));
        }
        let shape = (members.len(), longest);
        let ids = Tensor::from_vec(ids, shape, &Device::Cpu).map_err(|e| failed(&e))?;
        let mask = Tensor::from_vec(mask, shape, &Device::Cpu).map_err(|e| failed(&e))?;
        let types = ids.zeros_like().map_err(|e| failed(&e))?;
        let output = embedder.model.forward(&ids, &types, Some(&mask)).map_err(|e| failed(&e))?;
        // bge represents a whole text by its first ([CLS]) token.
        let rows: Vec<Vec<f32>> = output
            .i((.., 0))
            .and_then(|cls| cls.to_vec2())
            .map_err(|e| failed(&e))?;
        if rows.iter().any(|row| row.len() != DIMENSION) {
            return Err("The memory search model returned vectors of the wrong size.".into());
        }
        for (&i, row) in members.iter().zip(rows) {
            vectors[kept[i].0] = quantize(&row);
        }
    }
    loaded.last_used = Instant::now();
    Ok(vectors)
}

/// Splits token lengths, sorted shortest first, into runs of at most `MAX_BATCH` texts
/// within `ATTENTION_BUDGET`. A single text always runs, however long.
fn batches(lengths: &[usize]) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    while start < lengths.len() {
        let mut end = start + 1;
        while end < lengths.len()
            && end - start < MAX_BATCH
            && (end - start + 1) * lengths[end] * lengths[end] <= ATTENTION_BUDGET
        {
            end += 1;
        }
        groups.push(start..end);
        start = end;
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled() -> bool {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/bge-small-en-v1.5");
        if !dir.join("model.safetensors").is_file() {
            eprintln!("skipped: run scripts/fetch-models.sh to test the bundled model");
            return false;
        }
        set_model_dir(dir);
        true
    }

    #[test]
    fn vectors_are_unit_length_signed_bytes() {
        assert_eq!(quantize(&[3.0, -4.0]), Some(vec![76, -102]));
        assert_eq!(quantize(&[0.0, 0.0]), None);
        assert_eq!(quantize(&[f32::NAN]), None);
    }

    #[test]
    fn batches_are_small_and_long_texts_share_with_fewer() {
        assert_eq!(batches(&[10; 20]), [0..8, 8..16, 16..20]);
        assert_eq!(batches(&[400, 512, 512]), [0..2, 2..3]);
        assert_eq!(batches(&[512, 512, 512]), [0..2, 2..3]);
        assert_eq!(batches(&[]), Vec::<std::ops::Range<usize>>::new());
        for group in batches(&[100, 200, 300, 400, 500, 512, 512]) {
            let longest = [100, 200, 300, 400, 500, 512, 512][group.end - 1];
            assert!(group.len() == 1 || group.len() * longest * longest <= ATTENTION_BUDGET);
        }
    }

    /// Runs against the bundled model when scripts/fetch-models.sh has installed it.
    #[test]
    fn the_bundled_model_puts_a_question_near_its_answer() {
        if !bundled() {
            return;
        }
        let passages = [
            "Rental contract: sign the renewal for the flat by Friday".to_string(),
            "Banana bread recipe with walnuts".to_string(),
            "Flights to Lisbon on the 12th, hotel near the river".to_string(),
            "   ".to_string(),
        ];
        let documents = embed(&passages, false).unwrap();
        assert_eq!(documents[0].as_ref().unwrap().len(), DIMENSION);
        assert!(documents[3].is_none());
        let score = |query: &str, index: usize| {
            let q = embed(&[query.to_string()], true).unwrap().remove(0).unwrap();
            q.iter().zip(documents[index].as_ref().unwrap()).map(|(a, b)| i32::from(*a) * i32::from(*b)).sum::<i32>()
        };
        assert!(score("when is my lease renewal due", 0) > score("when is my lease renewal due", 1));
        assert!(score("when is my lease renewal due", 0) > score("when is my lease renewal due", 2));
        assert!(score("travel plans", 2) > score("travel plans", 0));
        assert!(score("travel plans", 2) > score("travel plans", 1));
    }

    #[test]
    fn a_text_embeds_the_same_alone_and_padded_in_a_batch() {
        if !bundled() {
            return;
        }
        let short = "Ana says the lease renewal is due Friday".to_string();
        // Longer, so the short text is padded to its length in the same batch.
        let long = "Packing list for the Lisbon trip: passport, adapters, walking shoes, rain jacket, the hotel booking printout, chargers for the phone and laptop, sunglasses, a small umbrella and a paperback for the flight. ".repeat(2);
        assert_eq!(batches(&[12, 100]), [0..2]);
        let alone = embed(std::slice::from_ref(&short), false).unwrap().remove(0).unwrap();
        let together = embed(&[long, short.clone()], false).unwrap();
        let padded = together[1].as_ref().unwrap();
        let similarity = alone.iter().zip(padded).map(|(a, b)| i32::from(*a) * i32::from(*b)).sum::<i32>() as f64 / (127.0 * 127.0);
        assert!(similarity >= 0.98, "padding changed the vector: {similarity}");
    }

    #[test]
    fn the_model_is_freed_when_idle_and_loads_again() {
        if !bundled() {
            return;
        }
        embed(&["Banana bread".to_string()], false).unwrap();
        assert!(!release_if_idle(Duration::from_secs(3600)), "just used, so it stays loaded");
        // Other tests may be using the model; it is freed as soon as they finish.
        let mut freed = false;
        for _ in 0..600 {
            if release_if_idle(Duration::ZERO) {
                freed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(freed);
        assert!(embed(&["Banana bread".to_string()], false).unwrap()[0].is_some(), "loads again when needed");
    }

    #[test]
    fn a_question_does_not_wait_for_indexing() {
        if !bundled() {
            return;
        }
        let held = MODEL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let started = Instant::now();
        let answer = std::thread::scope(|scope| {
            scope
                .spawn(|| embed_within(&["lease".to_string()], true, Duration::from_millis(300)))
                .join()
                .unwrap()
        });
        drop(held);
        assert_eq!(answer, Err(BUSY.to_string()));
        assert!(started.elapsed() < Duration::from_millis(450));
        assert!(embed(&["lease".to_string()], true).unwrap()[0].is_some());
    }
}
