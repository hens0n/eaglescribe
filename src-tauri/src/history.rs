//! Local transcript history: last N successful dictations / command rewrites.
//!
//! Stored only on disk under the app data dir. Never uploaded.

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default cap for retained entries.
pub const DEFAULT_HISTORY_MAX: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Stable id (unix ms + short counter suffix).
    pub id: String,
    /// Unix epoch milliseconds when the entry was recorded.
    pub at_ms: u64,
    /// `dictation` or `command`.
    pub kind: String,
    /// Final text that was injected / copied.
    pub text: String,
    /// Raw STT (or command instruction) when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryBook {
    #[serde(default)]
    pub entries: Vec<HistoryEntry>,
}

impl HistoryBook {
    pub fn load_or_default(path: &Path) -> Self {
        match Self::load(path) {
            Ok(h) => h,
            Err(_) => Self::default(),
        }
    }

    pub fn load(path: &Path) -> AppResult<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)
            .map_err(|e| AppError::from(format!("Read history failed: {e}")))?;
        if data.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(&data)
            .map_err(|e| AppError::from(format!("Parse history failed: {e}")))
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::from(format!("Create history dir failed: {e}")))?;
        }
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::from(format!("Serialize history failed: {e}")))?;
        fs::write(path, data).map_err(|e| AppError::from(format!("Write history failed: {e}")))?;
        Ok(())
    }

    /// Newest-first list for the UI (does not mutate storage order).
    pub fn list_newest_first(&self) -> Vec<HistoryEntry> {
        let mut entries = self.entries.clone();
        entries.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then_with(|| b.id.cmp(&a.id)));
        entries
    }

    /// Push a successful result; keep at most `max` newest entries.
    pub fn push(&mut self, kind: &str, text: &str, raw: Option<&str>, max: usize) {
        let text = text.trim();
        if text.is_empty() || max == 0 {
            return;
        }
        let at_ms = now_ms();
        let id = format!("{at_ms}-{}", self.entries.len() % 1000);
        self.entries.push(HistoryEntry {
            id,
            at_ms,
            kind: kind.to_string(),
            text: text.to_string(),
            raw: raw
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        });
        self.trim_to(max);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn trim_to(&mut self, max: usize) {
        if max == 0 {
            self.entries.clear();
            return;
        }
        if self.entries.len() <= max {
            return;
        }
        // Keep newest by at_ms.
        self.entries
            .sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then_with(|| a.id.cmp(&b.id)));
        let drop_n = self.entries.len() - max;
        self.entries.drain(0..drop_n);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn default_history_path() -> PathBuf {
    if let Some(data) = dirs::data_local_dir() {
        return data.join("eaglescribe").join("history.json");
    }
    PathBuf::from("history.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_respects_max_keeps_newest() {
        let mut h = HistoryBook::default();
        for i in 0..5 {
            h.push("dictation", &format!("text {i}"), None, 3);
            // Ensure distinct timestamps if the loop is too fast
            h.entries.last_mut().unwrap().at_ms = i as u64;
        }
        h.trim_to(3);
        assert_eq!(h.entries.len(), 3);
        let newest = h.list_newest_first();
        assert_eq!(newest[0].text, "text 4");
        assert_eq!(newest[2].text, "text 2");
    }

    #[test]
    fn clear_empties() {
        let mut h = HistoryBook::default();
        h.push("command", "hello", Some("raw"), 10);
        assert_eq!(h.entries.len(), 1);
        h.clear();
        assert!(h.entries.is_empty());
    }

    #[test]
    fn skips_empty_text() {
        let mut h = HistoryBook::default();
        h.push("dictation", "   ", None, 10);
        assert!(h.entries.is_empty());
    }
}
