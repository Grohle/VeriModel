//! Report-language strings. The UI has its own i18n JSON (frontend/i18n);
//! this module serves the generated documents (report.html, BCF titles).
//! Strings live in `data/i18n/<lang>.json`, never in code.

use std::collections::BTreeMap;

use crate::config::Language;

const EN: &str = include_str!("../data/i18n/en.json");
const ES: &str = include_str!("../data/i18n/es.json");

/// A loaded string table.
#[derive(Debug, Clone)]
pub struct Strings {
    table: BTreeMap<String, String>,
    fallback: BTreeMap<String, String>,
}

impl Strings {
    pub fn for_language(lang: Language) -> Strings {
        let en: BTreeMap<String, String> = serde_json::from_str(EN).expect("en.json must be valid");
        let table = match lang {
            Language::En => en.clone(),
            Language::Es => serde_json::from_str(ES).expect("es.json must be valid"),
        };
        Strings {
            table,
            fallback: en,
        }
    }

    /// Look up a key; falls back to English, then to the key itself (never panics, never empty).
    pub fn get(&self, key: &str) -> String {
        self.table
            .get(key)
            .or_else(|| self.fallback.get(key))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    /// Look up and substitute `{name}` placeholders.
    pub fn fmt(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut s = self.get(key);
        for (k, v) in args {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_languages_load_and_share_keys() {
        let en: BTreeMap<String, String> = serde_json::from_str(EN).unwrap();
        let es: BTreeMap<String, String> = serde_json::from_str(ES).unwrap();
        for k in en.keys() {
            assert!(es.contains_key(k), "es.json is missing key {k}");
        }
        let s = Strings::for_language(Language::Es);
        assert_ne!(s.get("report.title"), "report.title");
        assert_eq!(s.get("does.not.exist"), "does.not.exist");
    }
}
