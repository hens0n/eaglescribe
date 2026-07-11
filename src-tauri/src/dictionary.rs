//! Personal dictionary: force preferred spellings for names, brands, jargon.
//!
//! Stored locally as JSON under the app data directory. Applied after polish
//! with case-insensitive whole-word (or multi-word phrase) matching.

use crate::error::{AppError, AppResult};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictEntry {
    /// What STT typically produces (or a phrase to match), e.g. "eagle scribe".
    pub from: String,
    /// Preferred spelling to insert, e.g. "EagleScribe".
    pub to: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dictionary {
    pub entries: Vec<DictEntry>,
}

impl Dictionary {
    pub fn load_or_default(path: &Path) -> Self {
        match Self::load(path) {
            Ok(d) => d,
            Err(_) => Self::default(),
        }
    }

    pub fn load(path: &Path) -> AppResult<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)
            .map_err(|e| AppError::from(format!("Read dictionary failed: {e}")))?;
        if data.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(&data)
            .map_err(|e| AppError::from(format!("Parse dictionary failed: {e}")))
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::from(format!("Create dictionary dir failed: {e}")))?;
        }
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::from(format!("Serialize dictionary failed: {e}")))?;
        fs::write(path, data)
            .map_err(|e| AppError::from(format!("Write dictionary failed: {e}")))?;
        Ok(())
    }

    pub fn list(&self) -> Vec<DictEntry> {
        let mut entries = self.entries.clone();
        entries.sort_by(|a, b| {
            a.from
                .to_ascii_lowercase()
                .cmp(&b.from.to_ascii_lowercase())
        });
        entries
    }

    /// Upsert by case-insensitive `from` key.
    pub fn upsert(&mut self, from: &str, to: &str) -> AppResult<()> {
        let from = normalize_key(from)?;
        let to = to.trim();
        if to.is_empty() {
            return Err(AppError::from("Replacement text is empty"));
        }
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|e| e.from.eq_ignore_ascii_case(&from))
        {
            existing.from = from;
            existing.to = to.to_string();
        } else {
            self.entries.push(DictEntry {
                from,
                to: to.to_string(),
            });
        }
        Ok(())
    }

    /// Remove by case-insensitive `from` key. Returns true if something was removed.
    pub fn remove(&mut self, from: &str) -> bool {
        let needle = from.trim();
        let before = self.entries.len();
        self.entries
            .retain(|e| !e.from.eq_ignore_ascii_case(needle));
        self.entries.len() != before
    }

    /// Apply all entries to `text`. Longer `from` phrases run first.
    pub fn apply(&self, text: &str) -> String {
        if self.entries.is_empty() || text.is_empty() {
            return text.to_string();
        }

        let mut ordered = self.entries.clone();
        ordered.sort_by(|a, b| {
            b.from
                .chars()
                .count()
                .cmp(&a.from.chars().count())
                .then_with(|| a.from.cmp(&b.from))
        });

        let mut out = text.to_string();
        for entry in &ordered {
            out = replace_phrase(&out, &entry.from, &entry.to);
        }
        out
    }
}

fn normalize_key(from: &str) -> AppResult<String> {
    let from = from.split_whitespace().collect::<Vec<_>>().join(" ");
    if from.is_empty() {
        return Err(AppError::from("Dictionary key is empty"));
    }
    Ok(from)
}

/// Case-insensitive whole-phrase replace; preserves simple capitalization cues.
fn replace_phrase(text: &str, from: &str, to: &str) -> String {
    let from = from.trim();
    let to = to.trim();
    if from.is_empty() || to.is_empty() {
        return text.to_string();
    }

    let pattern = format!(r"(?i)\b{}\b", regex::escape(from));
    let Ok(re) = Regex::new(&pattern) else {
        return text.to_string();
    };

    re.replace_all(text, |caps: &regex::Captures| {
        let matched = caps.get(0).map(|m| m.as_str()).unwrap_or(from);
        adapt_casing(matched, to)
    })
    .into_owned()
}

/// If the STT match is ALL CAPS or Title Case, nudge the replacement similarly.
fn adapt_casing(matched: &str, replacement: &str) -> String {
    if matched
        .chars()
        .all(|c| !c.is_alphabetic() || c.is_uppercase())
        && matched.chars().any(|c| c.is_alphabetic())
    {
        return replacement.to_uppercase();
    }
    let mut chars = matched.chars();
    if let (Some(first), rest) = (chars.next(), chars.as_str()) {
        if first.is_uppercase() && rest.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()) {
            // Title-ish single word: capitalize first letter of replacement only
            // when replacement is all-lowercase.
            if replacement
                .chars()
                .all(|c| !c.is_alphabetic() || c.is_lowercase())
            {
                let mut r = replacement.chars();
                if let Some(rf) = r.next() {
                    return format!("{}{}", rf.to_uppercase(), r.as_str());
                }
            }
        }
    }
    replacement.to_string()
}

/// Default path: `~/Library/Application Support/eaglescribe/dictionary.json` (macOS)
/// or platform equivalent via `dirs::data_local_dir`.
pub fn default_dictionary_path() -> PathBuf {
    if let Some(data) = dirs::data_local_dir() {
        return data.join("eaglescribe").join("dictionary.json");
    }
    PathBuf::from("dictionary.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_phrase_case_insensitive() {
        let mut d = Dictionary::default();
        d.upsert("eagle scribe", "EagleScribe").unwrap();
        assert_eq!(
            d.apply("I love eagle scribe so much"),
            "I love EagleScribe so much"
        );
        assert_eq!(d.apply("EAGLE SCRIBE rocks"), "EAGLESCRIBE rocks");
    }

    #[test]
    fn longer_phrase_wins() {
        let mut d = Dictionary::default();
        d.upsert("type", "TYPE").unwrap();
        d.upsert("eagle scribe", "EagleScribe").unwrap();
        assert_eq!(d.apply("use eagle scribe please"), "use EagleScribe please");
    }

    #[test]
    fn upsert_and_remove() {
        let mut d = Dictionary::default();
        d.upsert("foo", "Foo").unwrap();
        d.upsert("FOO", "FOO!").unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].to, "FOO!");
        assert!(d.remove("foo"));
        assert!(!d.remove("foo"));
        assert!(d.entries.is_empty());
    }

    #[test]
    fn word_boundaries() {
        let mut d = Dictionary::default();
        d.upsert("cat", "feline").unwrap();
        assert_eq!(d.apply("the cat scattered"), "the feline scattered");
    }
}
