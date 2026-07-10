//! Global hotkey string parsing, validation, and defaults.
//!
//! Strings use the format accepted by `global-hotkey` / Tauri
//! (`Ctrl+Shift+Space`, `Cmd+Alt+KeyD`, …). Modifiers first, then one key.

use crate::error::{AppError, AppResult};
use std::str::FromStr;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

pub const DEFAULT_DICTATION_HOTKEY: &str = "Ctrl+Shift+Space";
/// Must not use `C` as the main key — selection capture synthesizes Cmd/Ctrl+C.
pub const DEFAULT_COMMAND_HOTKEY: &str = "Ctrl+Shift+X";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyRole {
    Dictation,
    Command,
}

impl HotkeyRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dictation => "dictation",
            Self::Command => "command mode",
        }
    }
}

/// Parse a hotkey string into a global shortcut.
pub fn parse_shortcut(s: &str) -> AppResult<Shortcut> {
    let s = s.trim();
    if s.is_empty() {
        return Err(AppError::from("Hotkey is empty"));
    }
    Shortcut::from_str(s).map_err(|e| {
        AppError::from(format!(
            "Invalid hotkey '{s}': {e}. Example: Ctrl+Shift+Space"
        ))
    })
}

/// Normalize whitespace around `+` tokens.
pub fn normalize_combo(s: &str) -> String {
    s.split('+')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("+")
}

/// Human-readable form for UI (e.g. `Ctrl+Shift+Space`).
pub fn display_combo(shortcut: &Shortcut) -> String {
    let mut parts: Vec<String> = Vec::new();
    if shortcut.mods.contains(Modifiers::CONTROL) {
        parts.push("Ctrl".into());
    }
    if shortcut.mods.contains(Modifiers::SUPER) {
        parts.push("Cmd".into());
    }
    if shortcut.mods.contains(Modifiers::ALT) {
        parts.push("Alt".into());
    }
    if shortcut.mods.contains(Modifiers::SHIFT) {
        parts.push("Shift".into());
    }
    parts.push(code_label(shortcut.key));
    parts.join("+")
}

fn code_label(code: Code) -> String {
    match code {
        Code::Space => "Space".into(),
        Code::Enter => "Enter".into(),
        Code::Tab => "Tab".into(),
        Code::Escape => "Esc".into(),
        Code::Backspace => "Backspace".into(),
        Code::Delete => "Delete".into(),
        Code::ArrowUp => "Up".into(),
        Code::ArrowDown => "Down".into(),
        Code::ArrowLeft => "Left".into(),
        Code::ArrowRight => "Right".into(),
        Code::KeyA => "A".into(),
        Code::KeyB => "B".into(),
        Code::KeyC => "C".into(),
        Code::KeyD => "D".into(),
        Code::KeyE => "E".into(),
        Code::KeyF => "F".into(),
        Code::KeyG => "G".into(),
        Code::KeyH => "H".into(),
        Code::KeyI => "I".into(),
        Code::KeyJ => "J".into(),
        Code::KeyK => "K".into(),
        Code::KeyL => "L".into(),
        Code::KeyM => "M".into(),
        Code::KeyN => "N".into(),
        Code::KeyO => "O".into(),
        Code::KeyP => "P".into(),
        Code::KeyQ => "Q".into(),
        Code::KeyR => "R".into(),
        Code::KeyS => "S".into(),
        Code::KeyT => "T".into(),
        Code::KeyU => "U".into(),
        Code::KeyV => "V".into(),
        Code::KeyW => "W".into(),
        Code::KeyX => "X".into(),
        Code::KeyY => "Y".into(),
        Code::KeyZ => "Z".into(),
        Code::Digit0 => "0".into(),
        Code::Digit1 => "1".into(),
        Code::Digit2 => "2".into(),
        Code::Digit3 => "3".into(),
        Code::Digit4 => "4".into(),
        Code::Digit5 => "5".into(),
        Code::Digit6 => "6".into(),
        Code::Digit7 => "7".into(),
        Code::Digit8 => "8".into(),
        Code::Digit9 => "9".into(),
        Code::F1 => "F1".into(),
        Code::F2 => "F2".into(),
        Code::F3 => "F3".into(),
        Code::F4 => "F4".into(),
        Code::F5 => "F5".into(),
        Code::F6 => "F6".into(),
        Code::F7 => "F7".into(),
        Code::F8 => "F8".into(),
        Code::F9 => "F9".into(),
        Code::F10 => "F10".into(),
        Code::F11 => "F11".into(),
        Code::F12 => "F12".into(),
        // Debug names like `KeyX` / `F13` still parse via FromStr.
        other => format!("{other:?}"),
    }
}

/// Validate one binding. Returns a normalized display string for storage.
pub fn validate_binding(raw: &str, role: HotkeyRole) -> AppResult<String> {
    let normalized = normalize_combo(raw);
    let sc = parse_shortcut(&normalized)?;

    if sc.mods.is_empty() {
        return Err(AppError::from(format!(
            "{} hotkey must include at least one modifier (Ctrl, Cmd, Alt, or Shift)",
            role.label()
        )));
    }

    if role == HotkeyRole::Command && sc.key == Code::KeyC {
        return Err(AppError::from(
            "Command Mode hotkey cannot use the C key — selection capture sends Cmd/Ctrl+C and would end the session immediately",
        ));
    }

    // Prefer stable, human-friendly storage.
    Ok(display_combo(&sc))
}

/// Validate both bindings and ensure they do not collide.
pub fn validate_pair(dictation: &str, command: &str) -> AppResult<(String, String)> {
    let dictation = validate_binding(dictation, HotkeyRole::Dictation)?;
    let command = validate_binding(command, HotkeyRole::Command)?;

    let d = parse_shortcut(&dictation)?;
    let c = parse_shortcut(&command)?;
    if d.id() == c.id() {
        return Err(AppError::from(format!(
            "Dictation and Command Mode hotkeys cannot be the same ({dictation})"
        )));
    }

    Ok((dictation, command))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defaults() {
        assert!(parse_shortcut(DEFAULT_DICTATION_HOTKEY).is_ok());
        assert!(parse_shortcut(DEFAULT_COMMAND_HOTKEY).is_ok());
    }

    #[test]
    fn rejects_empty_and_modifier_only() {
        assert!(validate_binding("", HotkeyRole::Dictation).is_err());
        assert!(validate_binding("Ctrl+Shift", HotkeyRole::Dictation).is_err());
    }

    #[test]
    fn rejects_no_modifier() {
        assert!(validate_binding("Space", HotkeyRole::Dictation).is_err());
        assert!(validate_binding("KeyX", HotkeyRole::Command).is_err());
    }

    #[test]
    fn rejects_command_key_c() {
        assert!(validate_binding("Ctrl+Shift+C", HotkeyRole::Command).is_err());
        assert!(validate_binding("Ctrl+Shift+X", HotkeyRole::Command).is_ok());
    }

    #[test]
    fn rejects_identical_pair() {
        assert!(validate_pair("Ctrl+Shift+Space", "Ctrl+Shift+Space").is_err());
        assert!(validate_pair(DEFAULT_DICTATION_HOTKEY, DEFAULT_COMMAND_HOTKEY).is_ok());
    }

    #[test]
    fn display_is_stable() {
        let sc = parse_shortcut("ctrl+shift+space").unwrap();
        assert_eq!(display_combo(&sc), "Ctrl+Shift+Space");
    }

    #[test]
    fn normalize_whitespace() {
        assert_eq!(normalize_combo(" Ctrl + Shift + Space "), "Ctrl+Shift+Space");
    }
}
