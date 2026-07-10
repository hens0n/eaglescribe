//! Lightweight local preferences (hotkey mode, etc.).

use crate::error::{AppError, AppResult};
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
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkey_mode: HotkeyMode::Hold,
        }
    }
}

impl AppSettings {
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
        return data.join("talontype").join("settings.json");
    }
    PathBuf::from("settings.json")
}
