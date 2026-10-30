//! Sentence vectors for the memory index: BAAI's bge-small-en-v1.5 (MIT), a small
//! transformer trained to put a question near the passage that answers it. It ships with
//! the app at half precision (scripts/fetch-models.sh) and runs in this process on the CPU
//! with Apple's Accelerate framework, whatever brain is chosen. Nothing is downloaded at
//! run time and no text leaves the Mac. Calls are synchronous; run them on a blocking thread.
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

/// Names the vectors this model makes. Stored vectors from any other model are redone.
pub const MODEL_ID: &str = "bge-small-en-v1.5@5c38ec7";
pub const DIMENSION: usize = 384;
/// Index records are at most 1,500 characters, well inside this.
const MAX_TOKENS: usize = 512;
/// bge v1.5 matches short questions to passages better with this prefix on the question.
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
}

static DIR: OnceLock<PathBuf> = OnceLock::new();
static MODEL: OnceLock<Result<Mutex<Embedder>, String>> = OnceLock::new();

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
    tokenizer.with_padding(Some(PaddingParams::default()));
    tokenizer
        .with_truncation(Some(TruncationParams { max_length: MAX_TOKENS, ..Default::default() }))
        .map_err(|e| format!("The memory search tokenizer couldn't load: {e}"))?;
    let weights = dir.join("model.safetensors");
    if !weights.is_file() {
        return Err(missing());
    }
    // SAFETY: the bundled file is read-only for the app's lifetime; the map is never written.
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights], DType::F32, &Device::Cpu) }
        .map_err(|e| format!("The memory search model couldn't load: {e}"))?;
    let model = BertModel::load(vb, &config).map_err(|e| format!("The memory search model couldn't load: {e}"))?;
    Ok(Embedder { model, tokenizer })
}

/// A unit-length vector as one signed byte per dimension (value × 127).
pub fn quantize(vector: &[f32]) -> Option<Vec<i8>> {
    let length = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    (length.is_finite() && length > 0.0)
        .then(|| vector.iter().map(|v| (v / length * 127.0).round().clamp(-127.0, 127.0) as i8).collect())
}

/// One vector per text (None for blank text). `query` marks a search question rather
/// than a record being indexed.
pub fn embed(texts: &[String], query: bool) -> Result<Vec<Option<Vec<i8>>>, String> {
    let dir = DIR.get().ok_or("Buntini's memory search model isn't set up.")?;
    let embedder = MODEL
        .get_or_init(|| load(dir).map(Mutex::new))
        .as_ref()
        .map_err(Clone::clone)?
        .lock()
        .unwrap();
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
    let failed = |e: &dyn std::fmt::Display| format!("Memory search couldn't read this text: {e}");
    let encodings = embedder
        .tokenizer
        .encode_batch(kept.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>(), true)
        .map_err(|e| failed(&e))?;
    let stack = |rows: Vec<&[u32]>| -> Result<Tensor, String> {
        let rows = rows
            .into_iter()
            .map(|row| Tensor::new(row, &Device::Cpu))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| failed(&e))?;
        Tensor::stack(&rows, 0).map_err(|e| failed(&e))
    };
    let ids = stack(encodings.iter().map(|e| e.get_ids()).collect())?;
    let mask = stack(encodings.iter().map(|e| e.get_attention_mask()).collect())?;
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
    for ((index, _), row) in kept.iter().zip(rows) {
        vectors[*index] = quantize(&row);
    }
    Ok(vectors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_are_unit_length_signed_bytes() {
        assert_eq!(quantize(&[3.0, -4.0]), Some(vec![76, -102]));
        assert_eq!(quantize(&[0.0, 0.0]), None);
        assert_eq!(quantize(&[f32::NAN]), None);
    }

    /// Runs against the bundled model when scripts/fetch-models.sh has installed it.
    #[test]
    fn the_bundled_model_puts_a_question_near_its_answer() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/bge-small-en-v1.5");
        if !dir.join("model.safetensors").is_file() {
            eprintln!("skipped: run scripts/fetch-models.sh to test the bundled model");
            return;
        }
        set_model_dir(dir);
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
}
