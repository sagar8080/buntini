//! Buntini's words in the person's language: one catalog per language in `locales/`, the same
//! files the React app reads. The native surfaces get theirs through the scene (`words`).
//!
//! - **Which language:** the one picked in Settings, else the first of the Mac's preferred
//!   languages Buntini has, else English.
//! - **Keys:** dotted, `area.name`. A key missing from a catalog falls back to English.
//! - **Placeholders:** `{name}`. A count picks a plural form, `key.one`, `key.few`, …,
//!   falling back to `key.other`.
use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::sync::{Mutex, OnceLock, RwLock};

/// The languages Buntini speaks, as Coucou does, English first.
pub const LANGUAGES: [&str; 16] = ["en", "zh-Hans", "zh-Hant", "hi", "es", "ar", "fr", "bn", "pt-BR", "ru", "id", "de", "tr", "th", "ko", "ja"];

const CATALOGS: [&str; 16] = [
    include_str!("../../locales/en.json"),
    include_str!("../../locales/zh-Hans.json"),
    include_str!("../../locales/zh-Hant.json"),
    include_str!("../../locales/hi.json"),
    include_str!("../../locales/es.json"),
    include_str!("../../locales/ar.json"),
    include_str!("../../locales/fr.json"),
    include_str!("../../locales/bn.json"),
    include_str!("../../locales/pt-BR.json"),
    include_str!("../../locales/ru.json"),
    include_str!("../../locales/id.json"),
    include_str!("../../locales/de.json"),
    include_str!("../../locales/tr.json"),
    include_str!("../../locales/th.json"),
    include_str!("../../locales/ko.json"),
    include_str!("../../locales/ja.json"),
];

static PARSED: [OnceLock<HashMap<String, String>>; 16] = [const { OnceLock::new() }; 16];
static ACTIVE: RwLock<&str> = RwLock::new("en");
type Listener = Box<dyn Fn(&'static str) + Send + Sync>;
static LISTENERS: Mutex<Vec<Listener>> = Mutex::new(Vec::new());

fn index(language: &str) -> usize {
    LANGUAGES.iter().position(|code| *code == language).unwrap_or(0)
}

fn catalog(language: &str) -> &'static HashMap<String, String> {
    let at = index(language);
    PARSED[at].get_or_init(|| serde_json::from_str(CATALOGS[at]).unwrap_or_default())
}

/// The language Buntini speaks now.
pub fn language() -> &'static str {
    *ACTIVE.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Written right to left.
pub fn rtl(language: &str) -> bool {
    language == "ar"
}

/// The language Buntini speaks when Settings leaves it to the Mac.
pub fn system() -> &'static str {
    resolve("", &mac_languages())
}

/// Follows Settings: `chosen` is a language from `LANGUAGES`, or empty for the Mac's own.
/// Tells whoever listens when the language changes.
pub fn follow(chosen: &str) {
    let next = resolve(chosen, &mac_languages());
    let previous = std::mem::replace(&mut *ACTIVE.write().unwrap_or_else(|poisoned| poisoned.into_inner()), next);
    if previous != next {
        for listener in LISTENERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter() {
            listener(next);
        }
    }
}

/// Called with the new language whenever it changes.
pub fn on_change(listener: impl Fn(&'static str) + Send + Sync + 'static) {
    LISTENERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(Box::new(listener));
}

/// The language picked in Settings, else the first of `preferred` Buntini has, else English.
pub fn resolve(chosen: &str, preferred: &[String]) -> &'static str {
    if let Some(code) = LANGUAGES.iter().find(|code| **code == chosen) {
        return code;
    }
    preferred.iter().find_map(|tag| matching(tag)).unwrap_or("en")
}

/// The language Buntini has for a BCP 47 tag such as `de-AT`, `zh-Hant-TW`, `zh-HK` or `pt-PT`.
pub fn matching(tag: &str) -> Option<&'static str> {
    let tag = tag.replace('_', "-");
    if let Some(code) = LANGUAGES.iter().find(|code| code.eq_ignore_ascii_case(&tag)) {
        return Some(code);
    }
    let mut parts = tag.split('-');
    let base = parts.next()?.to_ascii_lowercase();
    let rest: Vec<String> = parts.map(str::to_ascii_lowercase).collect();
    match base.as_str() {
        "zh" => {
            let traditional = rest.iter().any(|part| part == "hant") || (!rest.iter().any(|part| part == "hans") && rest.iter().any(|part| ["tw", "hk", "mo"].contains(&part.as_str())));
            Some(if traditional { "zh-Hant" } else { "zh-Hans" })
        }
        "pt" => Some("pt-BR"),
        // Indonesian's old code.
        "in" => Some("id"),
        _ => LANGUAGES.iter().find(|code| **code == base).copied(),
    }
}

/// The Mac's preferred languages, most wanted first. Tests read none, so they speak English.
fn mac_languages() -> Vec<String> {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        use objc2_core_foundation::{CFArray, CFLocale, CFRetained, CFString};
        let Some(list) = CFLocale::preferred_languages() else { return Vec::new() };
        // SAFETY: CFLocaleCopyPreferredLanguages returns an array of CFStrings.
        let list: CFRetained<CFArray<CFString>> = unsafe { CFRetained::cast_unchecked(list) };
        list.iter().map(|tag| tag.to_string()).collect()
    }
    #[cfg(not(all(target_os = "macos", not(test))))]
    Vec::new()
}

/// The text for `key` in `language`, falling back to English. Every key in use is checked by
/// the tests, so None is a typo; the key itself then shows.
pub fn raw(language: &str, key: &str) -> Option<&'static str> {
    catalog(language).get(key).or_else(|| catalog("en").get(key)).map(String::as_str)
}

/// Puts each `{name}` in `template` from `args`; names not given stay as written.
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let found = after.find('}').and_then(|close| args.iter().find(|(name, _)| *name == &after[..close]).map(|(_, value)| (value, close)));
        match found {
            Some((value, close)) => {
                out.push_str(&value.to_string());
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The CLDR plural category of a whole number in `language`.
pub fn plural(language: &str, count: u64) -> &'static str {
    let (tens, hundreds) = (count % 10, count % 100);
    match language {
        "ar" => match (count, hundreds) {
            (0, _) => "zero",
            (1, _) => "one",
            (2, _) => "two",
            (_, 3..=10) => "few",
            (_, 11..=99) => "many",
            _ => "other",
        },
        "ru" => match (tens, hundreds) {
            (1, h) if h != 11 => "one",
            (2..=4, h) if !(12..=14).contains(&h) => "few",
            _ => "many",
        },
        "fr" | "pt-BR" if count <= 1 => "one",
        "fr" | "pt-BR" | "es" if count != 0 && count % 1_000_000 == 0 => "many",
        "hi" | "bn" if count <= 1 => "one",
        "en" | "de" | "es" | "tr" if count == 1 => "one",
        _ => "other",
    }
}

/// `key` in `language`, with `{count}` and the other placeholders filled.
pub fn counted(language: &str, key: &str, count: u64, args: &[(&str, &dyn Display)]) -> String {
    let own = catalog(language);
    let template = own
        .get(&format!("{key}.{}", plural(language, count)))
        .or_else(|| own.get(&format!("{key}.other")))
        .or_else(|| catalog("en").get(&format!("{key}.{}", plural("en", count))))
        .map_or(key, String::as_str);
    let mut all: Vec<(&str, &dyn Display)> = vec![("count", &count)];
    all.extend_from_slice(args);
    fill(template, &all)
}

/// The text for `key` now.
pub fn t(key: &str) -> String {
    raw(language(), key).unwrap_or(key).to_string()
}

/// The text for `key` now, with its placeholders filled.
pub fn tf(key: &str, args: &[(&str, &dyn Display)]) -> String {
    fill(raw(language(), key).unwrap_or(key), args)
}

/// The text for `key` now in the plural form `count` takes, `{count}` filled.
pub fn tn(key: &str, count: u64, args: &[(&str, &dyn Display)]) -> String {
    counted(language(), key, count, args)
}

/// The island tour's made-up text (the shared sample scenes are written in English) in the
/// language now: `text` is the English of a `sample.*` key.
pub fn sample(text: &str) -> Option<String> {
    let key = catalog("en").iter().find(|(key, english)| key.starts_with("sample.") && english.as_str() == text)?.0;
    raw(language(), key).map(str::to_string)
}

/// Puts every made-up text in a sample (JSON) into the language now.
pub fn samples(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(said) = sample(text) {
                *text = said;
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(samples),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(samples),
        _ => {}
    }
}

/// The words the native surfaces draw, every `surface.*` key in `language`.
pub fn surface_words(language: &'static str) -> crate::surface::scene::Words {
    let labels: BTreeMap<String, String> = catalog("en").keys().filter(|key| key.starts_with("surface.")).map(|key| (key.clone(), raw(language, key).unwrap_or(key).to_string())).collect();
    crate::surface::scene::Words { language: language.into(), rtl: rtl(language), labels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    const PLURAL_FORMS: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

    /// The plural forms a language must have, and those it may.
    fn forms(language: &str) -> (&'static [&'static str], &'static [&'static str]) {
        match language {
            "ar" => (&["zero", "one", "two", "few", "many", "other"], &["zero", "one", "two", "few", "many", "other"]),
            "ru" => (&["one", "few", "many", "other"], &["one", "few", "many", "other"]),
            "fr" | "pt-BR" | "es" => (&["one", "other"], &["one", "many", "other"]),
            "en" | "de" | "tr" | "hi" | "bn" => (&["one", "other"], &["one", "other"]),
            _ => (&["other"], &["other"]),
        }
    }

    fn placeholders(text: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('}') else { break };
            let name = &after[..close];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                found.insert(name.to_string());
            }
            rest = &after[close + 1..];
        }
        found
    }

    /// `key.one` → (`key`, `one`) for plural forms.
    fn plural_base(key: &str) -> Option<(&str, &str)> {
        let (base, form) = key.rsplit_once('.')?;
        PLURAL_FORMS.contains(&form).then_some((base, form))
    }

    #[test]
    fn every_catalog_reads_and_has_every_key_with_the_same_placeholders() {
        let english = catalog("en");
        assert!(english.len() > 100, "the English catalog is the source");
        let plural_keys: BTreeSet<&str> = english.keys().filter_map(|key| plural_base(key).map(|(base, _)| base)).collect();
        for (at, language) in LANGUAGES.iter().enumerate() {
            let parsed: HashMap<String, String> = serde_json::from_str(CATALOGS[at]).unwrap_or_else(|problem| panic!("{language}.json doesn't read: {problem}"));
            let (must, may) = forms(language);
            for (key, text) in english {
                // Plural forms differ by language; checked below.
                if plural_base(key).is_some() {
                    continue;
                }
                let translated = parsed.get(key).unwrap_or_else(|| panic!("{language}.json has no {key}"));
                assert!(!translated.trim().is_empty(), "{language}.json leaves {key} empty");
                assert_eq!(placeholders(translated), placeholders(text), "{language}.json {key} has other placeholders");
            }
            for base in &plural_keys {
                let others = placeholders(&english[&format!("{base}.other")]);
                let wanted: BTreeSet<String> = others.iter().filter(|name| *name != "count").cloned().collect();
                for form in must {
                    assert!(parsed.contains_key(&format!("{base}.{form}")), "{language}.json has no {base}.{form}");
                }
                for form in PLURAL_FORMS {
                    let Some(text) = parsed.get(&format!("{base}.{form}")) else { continue };
                    assert!(may.contains(&form), "{language}.json has {base}.{form}, a form {language} doesn't use");
                    let found = placeholders(text);
                    let without_count: BTreeSet<String> = found.iter().filter(|name| *name != "count").cloned().collect();
                    assert_eq!(without_count, wanted, "{language}.json {base}.{form} has other placeholders");
                    assert!(found.is_subset(&others), "{language}.json {base}.{form} has a placeholder English doesn't");
                }
            }
            for key in parsed.keys() {
                let known = english.contains_key(key) || plural_base(key).is_some_and(|(base, _)| plural_keys.contains(base));
                assert!(known, "{language}.json has {key}, which English doesn't");
            }
        }
    }

    /// Every key the code asks for is in the English catalog: Rust's `t`, `tf` and `tn`, React's
    /// `t` and `tn`, and the native surfaces' labels.
    #[test]
    fn every_key_in_use_is_in_the_english_catalog() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let english = catalog("en");
        let exists = |key: &str| english.contains_key(key) || english.contains_key(&format!("{key}.other"));
        let mut missing = Vec::new();
        let mut used = 0;
        // `(pattern, how much of its end starts the key)`: the key runs on to the next quote.
        let mut look = |folder: &Path, extensions: &[&str], patterns: &[(&str, usize)]| {
            for entry in walkdir::WalkDir::new(folder).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                if path.components().any(|part| [".build", "node_modules", "target"].contains(&part.as_os_str().to_str().unwrap_or_default())) {
                    continue;
                }
                if !extensions.iter().any(|extension| path.extension().is_some_and(|found| found == *extension)) {
                    continue;
                }
                // Tests make up keys of their own.
                if path.to_string_lossy().ends_with(".test.ts") {
                    continue;
                }
                let text = std::fs::read_to_string(path).unwrap_or_default();
                for (pattern, kept) in patterns {
                    for (at, _) in text.match_indices(pattern) {
                        // `t(` itself, not `split(` or `.get(`.
                        if text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                            continue;
                        }
                        let start = at + pattern.len() - kept;
                        let Some(end) = text[start..].find(['"', '\'', '`']) else { continue };
                        let key = &text[start..start + end];
                        // A key built at run time is checked where it's built.
                        if !key.contains('.') || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) || text[start + end..].starts_with('`') {
                            continue;
                        }
                        used += 1;
                        if !exists(key) {
                            missing.push(format!("{}: {key}", path.strip_prefix(root).unwrap_or(path).display()));
                        }
                    }
                }
            }
        };
        look(&root.join("src-tauri/src"), &["rs"], &[("t(\"", 0), ("tf(\"", 0), ("tn(\"", 0)]);
        look(&root.join("src"), &["ts", "tsx"], &[("t('", 0), ("tn('", 0), ("t(\"", 0), ("tn(\"", 0)]);
        look(&root.join("native/surface/Sources"), &["swift"], &[("= \"surface.", "surface.".len())]);
        assert!(missing.is_empty(), "not in locales/en.json:\n{}", missing.join("\n"));
        assert!(used > 100, "only {used} keys found in use");
    }

    #[test]
    fn a_language_comes_from_settings_then_the_mac_then_english() {
        let mac = |tags: &[&str]| tags.iter().map(|tag| tag.to_string()).collect::<Vec<_>>();
        assert_eq!(resolve("de", &mac(&["fr-FR"])), "de", "Settings first");
        assert_eq!(resolve("", &mac(&["fr-FR", "en-US"])), "fr");
        assert_eq!(resolve("", &mac(&["nb-NO", "de-AT"])), "de", "the first the Mac prefers that Buntini has");
        assert_eq!(resolve("", &mac(&["nb-NO"])), "en");
        assert_eq!(resolve("", &[]), "en");
        assert_eq!(resolve("klingon", &mac(&["ja-JP"])), "ja", "an unknown choice follows the Mac");
        assert_eq!(matching("zh-Hant-TW"), Some("zh-Hant"));
        assert_eq!(matching("zh-HK"), Some("zh-Hant"));
        assert_eq!(matching("zh-Hans-HK"), Some("zh-Hans"));
        assert_eq!(matching("zh-CN"), Some("zh-Hans"));
        assert_eq!(matching("zh"), Some("zh-Hans"));
        assert_eq!(matching("pt-PT"), Some("pt-BR"));
        assert_eq!(matching("pt_BR"), Some("pt-BR"));
        assert_eq!(matching("es-419"), Some("es"));
        assert_eq!(matching("EN-gb"), Some("en"));
        assert_eq!(matching("ar-SA"), Some("ar"));
        assert_eq!(matching("in-ID"), Some("id"));
        assert_eq!(matching("sv-SE"), None);
    }

    #[test]
    fn a_missing_text_falls_back_to_english() {
        let key = catalog("en").keys().find(|key| key.starts_with("surface.") && plural_base(key).is_none()).unwrap().clone();
        assert_eq!(raw("en", &key), Some(catalog("en")[&key].as_str()));
        assert_eq!(raw("de", &key), Some(catalog("de")[&key].as_str()));
        // A language Buntini doesn't have reads as English.
        assert_eq!(raw("sv", &key), Some(catalog("en")[&key].as_str()));
        assert_eq!(raw("de", "no.such.key"), None);
        assert_eq!(fill(raw("de", "no.such.key").unwrap_or("no.such.key"), &[]), "no.such.key");
    }

    #[test]
    fn placeholders_are_filled_and_unknown_ones_kept() {
        assert_eq!(fill("Hi {name}", &[("name", &"Ana")]), "Hi Ana");
        assert_eq!(fill("{a} and {b}", &[("b", &2), ("a", &1)]), "1 and 2");
        assert_eq!(fill("{missing} {", &[]), "{missing} {");
        assert_eq!(fill("Übergröße {n}%", &[("n", &5)]), "Übergröße 5%");
        assert_eq!(fill("{n}{n}", &[("n", &"x")]), "xx");
    }

    #[test]
    fn plural_forms_follow_each_language() {
        let cases: &[(&str, u64, &str)] = &[
            ("en", 0, "other"), ("en", 1, "one"), ("en", 2, "other"),
            ("de", 1, "one"), ("de", 21, "other"),
            ("fr", 0, "one"), ("fr", 1, "one"), ("fr", 2, "other"), ("fr", 1_000_000, "many"),
            ("pt-BR", 0, "one"), ("pt-BR", 2, "other"),
            ("es", 1, "one"), ("es", 0, "other"), ("es", 2_000_000, "many"),
            ("hi", 0, "one"), ("hi", 1, "one"), ("hi", 2, "other"), ("bn", 1, "one"),
            ("tr", 1, "one"), ("tr", 5, "other"),
            ("ru", 1, "one"), ("ru", 21, "one"), ("ru", 11, "many"), ("ru", 2, "few"), ("ru", 24, "few"), ("ru", 12, "many"), ("ru", 5, "many"), ("ru", 0, "many"), ("ru", 111, "many"),
            ("ar", 0, "zero"), ("ar", 1, "one"), ("ar", 2, "two"), ("ar", 3, "few"), ("ar", 110, "few"), ("ar", 11, "many"), ("ar", 99, "many"), ("ar", 100, "other"), ("ar", 102, "other"),
            ("ja", 1, "other"), ("zh-Hans", 1, "other"), ("ko", 1, "other"), ("th", 1, "other"), ("id", 1, "other"), ("zh-Hant", 2, "other"),
        ];
        for (language, count, form) in cases {
            assert_eq!(plural(language, *count), *form, "{language} {count}");
        }
    }

    #[test]
    fn a_count_picks_its_form_and_falls_back_to_other() {
        let base = catalog("en").keys().find_map(|key| key.strip_suffix(".one")).expect("a plural key in English").to_string();
        let one = counted("en", &base, 1, &[]);
        let many = counted("en", &base, 7, &[]);
        assert_ne!(one, many);
        assert!(many.contains('7') || !catalog("en")[&format!("{base}.other")].contains("{count}"));
        // Russian 5 is "many"; Arabic 100 is "other".
        assert!(!counted("ru", &base, 5, &[]).is_empty());
        assert!(!counted("ar", &base, 100, &[]).is_empty());
    }

    #[test]
    fn the_surfaces_get_every_surface_word_in_the_language() {
        let words = surface_words("ar");
        assert!(words.rtl);
        assert_eq!(words.language, "ar");
        let english = surface_words("en");
        assert!(!english.rtl);
        assert_eq!(words.labels.keys().collect::<Vec<_>>(), english.labels.keys().collect::<Vec<_>>());
        assert!(english.labels.len() > 50);
        assert!(english.labels.keys().all(|key| key.starts_with("surface.")));
    }
}
