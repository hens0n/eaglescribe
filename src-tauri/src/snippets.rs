//! Voice snippets: speak a short cue, expand to a longer canned text block.
//!
//! Stored locally as JSON. Applied after dictionary so custom spellings can
//! appear inside expansions and cues.

use crate::error::{AppError, AppResult};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    /// Spoken / typed trigger, e.g. "calendar link".
    pub cue: String,
    /// Full text to insert (may contain newlines).
    pub expansion: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnippetBook {
    pub snippets: Vec<Snippet>,
}

impl SnippetBook {
    pub fn load_or_default(path: &Path) -> Self {
        match Self::load(path) {
            Ok(s) => s,
            Err(_) => Self::default(),
        }
    }

    pub fn load(path: &Path) -> AppResult<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)
            .map_err(|e| AppError::from(format!("Read snippets failed: {e}")))?;
        if data.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(&data)
            .map_err(|e| AppError::from(format!("Parse snippets failed: {e}")))
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::from(format!("Create snippets dir failed: {e}")))?;
        }
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::from(format!("Serialize snippets failed: {e}")))?;
        fs::write(path, data).map_err(|e| AppError::from(format!("Write snippets failed: {e}")))?;
        Ok(())
    }

    pub fn list(&self) -> Vec<Snippet> {
        let mut snippets = self.snippets.clone();
        snippets.sort_by(|a, b| {
            a.cue
                .to_ascii_lowercase()
                .cmp(&b.cue.to_ascii_lowercase())
        });
        snippets
    }

    pub fn upsert(&mut self, cue: &str, expansion: &str) -> AppResult<()> {
        let cue = normalize_cue(cue)?;
        let expansion = expansion.trim_end();
        // Allow leading newline in expansion; trim only trailing whitespace runs
        let expansion = expansion.trim_start_matches(|c: char| c == ' ' || c == '\t');
        if expansion.is_empty() {
            return Err(AppError::from("Snippet expansion is empty"));
        }
        if let Some(existing) = self
            .snippets
            .iter_mut()
            .find(|s| s.cue.eq_ignore_ascii_case(&cue))
        {
            existing.cue = cue;
            existing.expansion = expansion.to_string();
        } else {
            self.snippets.push(Snippet {
                cue,
                expansion: expansion.to_string(),
            });
        }
        Ok(())
    }

    pub fn remove(&mut self, cue: &str) -> bool {
        let needle = cue.trim();
        let before = self.snippets.len();
        self.snippets
            .retain(|s| !s.cue.eq_ignore_ascii_case(needle));
        self.snippets.len() != before
    }

    /// Expand cues in `text`. Longer cues first.
    ///
    /// If the entire utterance (ignoring outer punctuation/whitespace) equals a
    /// cue, the expansion replaces the whole string (including stripping a
    /// trailing auto-period from polish when appropriate).
    pub fn apply(&self, text: &str) -> (String, bool) {
        if self.snippets.is_empty() || text.is_empty() {
            return (text.to_string(), false);
        }

        let mut ordered = self.snippets.clone();
        ordered.sort_by(|a, b| {
            b.cue
                .chars()
                .count()
                .cmp(&a.cue.chars().count())
                .then_with(|| a.cue.cmp(&b.cue))
        });

        // Whole-utterance match first
        let core = strip_outer_punct(text);
        for snip in &ordered {
            if core.eq_ignore_ascii_case(&snip.cue) {
                return (snip.expansion.clone(), true);
            }
        }

        // In-place phrase expansion
        let mut out = text.to_string();
        let mut changed = false;
        for snip in &ordered {
            let next = replace_cue(&out, &snip.cue, &snip.expansion);
            if next != out {
                changed = true;
                out = next;
            }
        }
        (out, changed)
    }
}

fn normalize_cue(cue: &str) -> AppResult<String> {
    let cue = cue.split_whitespace().collect::<Vec<_>>().join(" ");
    if cue.is_empty() {
        return Err(AppError::from("Snippet cue is empty"));
    }
    Ok(cue)
}

fn strip_outer_punct(s: &str) -> String {
    s.trim()
        .trim_matches(|c: char| matches!(c, '.' | '!' | '?' | '…' | '"' | '\'' | ',' | ';'))
        .trim()
        .to_string()
}

fn replace_cue(text: &str, cue: &str, expansion: &str) -> String {
    let cue = cue.trim();
    if cue.is_empty() {
        return text.to_string();
    }
    let pattern = format!(r"(?i)\b{}\b", regex::escape(cue));
    let Ok(re) = Regex::new(&pattern) else {
        return text.to_string();
    };
    re.replace_all(text, expansion).into_owned()
}

pub fn default_snippets_path() -> PathBuf {
    if let Some(data) = dirs::data_local_dir() {
        return data.join("eaglescribe").join("snippets.json");
    }
    PathBuf::from("snippets.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_utterance_expands() {
        let mut book = SnippetBook::default();
        book.upsert(
            "calendar link",
            "You can book time here: https://cal.example/me",
        )
        .unwrap();
        let (out, changed) = book.apply("Calendar link.");
        assert!(changed);
        assert_eq!(out, "You can book time here: https://cal.example/me");
    }

    #[test]
    fn in_place_expands() {
        let mut book = SnippetBook::default();
        book.upsert("sig", "— Jacob").unwrap();
        let (out, changed) = book.apply("Thanks, sig");
        assert!(changed);
        assert_eq!(out, "Thanks, — Jacob");
    }

    #[test]
    fn longer_cue_wins() {
        let mut book = SnippetBook::default();
        book.upsert("link", "SHORT").unwrap();
        book.upsert("calendar link", "LONG").unwrap();
        let (out, _) = book.apply("calendar link");
        assert_eq!(out, "LONG");
    }

    #[test]
    fn upsert_remove() {
        let mut book = SnippetBook::default();
        book.upsert("a", "1").unwrap();
        book.upsert("A", "2").unwrap();
        assert_eq!(book.snippets.len(), 1);
        assert_eq!(book.snippets[0].expansion, "2");
        assert!(book.remove("a"));
        assert!(book.snippets.is_empty());
    }
}
