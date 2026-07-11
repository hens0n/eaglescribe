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
    /// After a successful paste, restore the previous clipboard text
    /// (INJ-04). When false, the injected transcript remains on the clipboard.
    #[serde(default = "default_true")]
    pub clipboard_restore: bool,
    /// When true, trim leading/trailing silence after stop before Whisper
    /// (STT-05). Default on; applies to dictation and Command Mode.
    #[serde(default = "default_true")]
    pub silence_trim: bool,
    /// macOS: when true, next launch uses Accessory activation policy (no Dock
    /// icon; menu-bar tray remains). Default off; applied only at process start.
    /// No-op on non-macOS builds (field still persists for cross-platform settings).
    #[serde(default)]
    pub menu_bar_only: bool,
    /// Preferred microphone device name from cpal enumeration.
    /// `None` or empty string = system default input (pre-feature behavior).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_device_name: Option<String>,
    /// When true, the first-run setup checklist does not auto-show on launch.
    /// Missing / corrupt → false (show checklist). Re-open from Settings still works.
    #[serde(default)]
    pub onboarding_dismissed: bool,
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
            clipboard_restore: true,
            silence_trim: true,
            menu_bar_only: false,
            input_device_name: None,
            onboarding_dismissed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_settings_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("eaglescribe-settings-{label}-{nanos}.json"))
    }

    #[test]
    fn default_enables_clipboard_restore() {
        let s = AppSettings::default();
        assert!(s.clipboard_restore);
    }

    #[test]
    fn missing_clipboard_restore_field_defaults_true() {
        // Older settings.json without the field must keep restore on.
        let path = unique_settings_path("legacy");
        fs::write(
            &path,
            r#"{"hotkey_mode":"hold","history_enabled":true,"history_max":50}"#,
        )
        .expect("write");
        let s = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert!(s.clipboard_restore);
    }

    #[test]
    fn clipboard_restore_false_roundtrips() {
        let path = unique_settings_path("off");
        let mut s = AppSettings::default();
        s.clipboard_restore = false;
        s.save(&path).expect("save");
        let loaded = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert!(!loaded.clipboard_restore);
    }

    #[test]
    fn default_enables_silence_trim() {
        let s = AppSettings::default();
        assert!(s.silence_trim);
    }

    #[test]
    fn missing_silence_trim_field_defaults_true() {
        // Older settings.json without the field must keep trim on.
        let path = unique_settings_path("legacy-trim");
        fs::write(
            &path,
            r#"{"hotkey_mode":"hold","history_enabled":true,"history_max":50}"#,
        )
        .expect("write");
        let s = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert!(s.silence_trim);
    }

    #[test]
    fn silence_trim_false_roundtrips() {
        let path = unique_settings_path("trim-off");
        let s = AppSettings {
            silence_trim: false,
            ..Default::default()
        };
        s.save(&path).expect("save");
        let loaded = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert!(!loaded.silence_trim);
    }

    #[test]
    fn corrupt_silence_trim_load_or_default_is_on() {
        // Unknown / non-bool value fails parse → load_or_default → on.
        let path = unique_settings_path("trim-corrupt");
        fs::write(&path, r#"{"silence_trim":"yes"}"#).expect("write");
        let s = AppSettings::load_or_default(&path);
        let _ = fs::remove_file(&path);
        assert!(s.silence_trim);
    }

    #[test]
    fn default_disables_menu_bar_only() {
        let s = AppSettings::default();
        assert!(!s.menu_bar_only);
    }

    #[test]
    fn missing_menu_bar_only_field_defaults_false() {
        // Older settings.json without the field must keep Dock present.
        let path = unique_settings_path("legacy-mbo");
        fs::write(
            &path,
            r#"{"hotkey_mode":"hold","history_enabled":true,"history_max":50}"#,
        )
        .expect("write");
        let s = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert!(!s.menu_bar_only);
    }

    #[test]
    fn menu_bar_only_true_roundtrips() {
        let path = unique_settings_path("mbo-on");
        let s = AppSettings {
            menu_bar_only: true,
            ..Default::default()
        };
        s.save(&path).expect("save");
        let loaded = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert!(loaded.menu_bar_only);
    }

    #[test]
    fn corrupt_menu_bar_only_load_or_default_is_off() {
        // Unknown / non-bool value fails parse → load_or_default → off.
        let path = unique_settings_path("mbo-corrupt");
        fs::write(&path, r#"{"menu_bar_only":"yes"}"#).expect("write");
        let s = AppSettings::load_or_default(&path);
        let _ = fs::remove_file(&path);
        assert!(!s.menu_bar_only);
    }

    #[test]
    fn default_input_device_is_none() {
        let s = AppSettings::default();
        assert!(s.input_device_name.is_none());
    }

    #[test]
    fn missing_input_device_field_defaults_none() {
        let path = unique_settings_path("legacy-mic");
        fs::write(
            &path,
            r#"{"hotkey_mode":"hold","history_enabled":true,"history_max":50}"#,
        )
        .expect("write");
        let s = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert!(s.input_device_name.is_none());
    }

    #[test]
    fn input_device_name_roundtrips() {
        let path = unique_settings_path("mic");
        let s = AppSettings {
            input_device_name: Some("USB Headset".into()),
            ..Default::default()
        };
        s.save(&path).expect("save");
        let loaded = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert_eq!(loaded.input_device_name.as_deref(), Some("USB Headset"));
    }

    #[test]
    fn empty_input_device_string_loads() {
        // Corrupt/empty preference should load without hard fail; callers treat as default.
        let path = unique_settings_path("mic-empty");
        fs::write(&path, r#"{"hotkey_mode":"hold","input_device_name":""}"#).expect("write");
        let s = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert_eq!(s.input_device_name.as_deref(), Some(""));
    }

    #[test]
    fn default_onboarding_not_dismissed() {
        let s = AppSettings::default();
        assert!(!s.onboarding_dismissed);
    }

    #[test]
    fn missing_onboarding_dismissed_field_defaults_false() {
        // Older settings.json without the field must show the first-run checklist.
        let path = unique_settings_path("legacy-onboarding");
        fs::write(
            &path,
            r#"{"hotkey_mode":"hold","history_enabled":true,"history_max":50}"#,
        )
        .expect("write");
        let s = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert!(!s.onboarding_dismissed);
    }

    #[test]
    fn onboarding_dismissed_true_roundtrips() {
        let path = unique_settings_path("onboarding-on");
        let s = AppSettings {
            onboarding_dismissed: true,
            ..Default::default()
        };
        s.save(&path).expect("save");
        let loaded = AppSettings::load(&path).expect("load");
        let _ = fs::remove_file(&path);
        assert!(loaded.onboarding_dismissed);
    }

    #[test]
    fn corrupt_onboarding_dismissed_load_or_default_is_false() {
        // Unknown / non-bool value fails parse → load_or_default → not dismissed (show).
        let path = unique_settings_path("onboarding-corrupt");
        fs::write(&path, r#"{"onboarding_dismissed":"yes"}"#).expect("write");
        let s = AppSettings::load_or_default(&path);
        let _ = fs::remove_file(&path);
        assert!(!s.onboarding_dismissed);
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
        fs::write(path, data).map_err(|e| AppError::from(format!("Write settings failed: {e}")))?;
        Ok(())
    }
}

pub fn default_settings_path() -> PathBuf {
    if let Some(data) = dirs::data_local_dir() {
        return data.join("eaglescribe").join("settings.json");
    }
    PathBuf::from("settings.json")
}
