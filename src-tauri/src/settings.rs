//! Lightweight local preferences (hotkey mode, bindings, LLM endpoint, etc.).

use crate::error::{AppError, AppResult};
use crate::hotkey::{DEFAULT_COMMAND_HOTKEY, DEFAULT_DICTATION_HOTKEY};
use crate::llm::LlmConfig;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    /// Press starts recording; release stops and transcribes.
    #[default]
    Hold,
    /// Each press toggles recording on/off (previous behavior).
    Toggle,
}

impl HotkeyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Toggle => "toggle",
        }
    }

    pub fn parse(s: &str) -> AppResult<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "hold" => Ok(Self::Hold),
            "toggle" => Ok(Self::Toggle),
            other => Err(AppError::from(format!(
                "Unknown hotkey mode '{other}' (use hold or toggle)"
            ))),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hold => "hold to talk · release to paste",
            Self::Toggle => "press to start · press again to paste",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub hotkey_mode: HotkeyMode,
    /// Global dictation chord, e.g. `Ctrl+Shift+Space`.
    #[serde(default = "default_dictation_hotkey")]
    pub dictation_hotkey: String,
    /// Global Command Mode chord, e.g. `Ctrl+Shift+X` (must not use C).
    #[serde(default = "default_command_hotkey")]
    pub command_hotkey: String,
    /// OpenAI-compatible base URL on localhost (Ollama / llama-server).
    #[serde(default = "default_llm_base_url")]
    pub llm_base_url: String,
    #[serde(default = "default_llm_model")]
    pub llm_model: String,
    #[serde(default)]
    pub llm_api_key: String,
    /// When true, successful injects are appended to `history.json`.
    #[serde(default = "default_true")]
    pub history_enabled: bool,
    /// Max retained history entries (newest kept).
    #[serde(default = "default_history_max")]
    pub history_max: usize,
}

fn default_true() -> bool {
    true
}

fn default_history_max() -> usize {
    crate::history::DEFAULT_HISTORY_MAX
}

fn default_dictation_hotkey() -> String {
    DEFAULT_DICTATION_HOTKEY.into()
}

fn default_command_hotkey() -> String {
    DEFAULT_COMMAND_HOTKEY.into()
}

fn default_llm_base_url() -> String {
    "http://127.0.0.1:11434/v1".into()
}

fn default_llm_model() -> String {
    "llama3.2".into()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkey_mode: HotkeyMode::Hold,
            dictation_hotkey: default_dictation_hotkey(),
            command_hotkey: default_command_hotkey(),
            llm_base_url: default_llm_base_url(),
            llm_model: default_llm_model(),
            llm_api_key: String::new(),
            history_enabled: true,
            history_max: default_history_max(),
        }
    }
}

impl AppSettings {
    pub fn llm_config(&self) -> LlmConfig {
        LlmConfig {
            base_url: self.llm_base_url.clone(),
            model: self.llm_model.clone(),
            api_key: self.llm_api_key.clone(),
        }
    }

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
            .map_err(|e| AppError::from(format!("Read settings failed: {e}")))?;
        if data.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(&data)
            .map_err(|e| AppError::from(format!("Parse settings failed: {e}")))
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::from(format!("Create settings dir failed: {e}")))?;
        }
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::from(format!("Serialize settings failed: {e}")))?;
        fs::write(path, data)
            .map_err(|e| AppError::from(format!("Write settings failed: {e}")))?;
        Ok(())
    }
}

pub fn default_settings_path() -> PathBuf {
    if let Some(data) = dirs::data_local_dir() {
        return data.join("eaglescribe").join("settings.json");
    }
    PathBuf::from("settings.json")
}
