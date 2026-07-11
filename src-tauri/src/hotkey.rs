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

/// Fixed cancel key while recording (not user-rebindable). Registered only when `recording`.
pub const ESCAPE_CANCEL_HOTKEY: &str = "Escape";

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

/// True when the shortcut is bare Escape (no modifiers) — reserved for cancel.
pub fn is_escape_alone(shortcut: &Shortcut) -> bool {
    shortcut.key == Code::Escape && shortcut.mods.is_empty()
}

/// Validate one binding. Returns a normalized display string for storage.
pub fn validate_binding(raw: &str, role: HotkeyRole) -> AppResult<String> {
    let normalized = normalize_combo(raw);
    let sc = parse_shortcut(&normalized)?;

    // Escape alone is reserved to cancel an active recording (see escape-cancel-spec).
    if is_escape_alone(&sc) {
        return Err(AppError::from(format!(
            "Cannot bind {} to Escape alone — Escape is reserved to cancel recording",
            role.label()
        )));
    }

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

/// Linux display session as reported by `$XDG_SESSION_TYPE` (best-effort).
///
/// Used for honest hotkey-failure messaging. Non-Linux builds always report
/// [`LinuxSession::Unknown`] (session type is not meaningful for this product).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxSession {
    X11,
    Wayland,
    /// Non-empty `$XDG_SESSION_TYPE` that is neither x11 nor wayland.
    Other,
    Unknown,
}

impl LinuxSession {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::X11 => "x11",
            Self::Wayland => "wayland",
            Self::Other => "other",
            Self::Unknown => "unknown",
        }
    }
}

/// Classify a raw `$XDG_SESSION_TYPE` value (testable without touching the environment).
pub fn classify_linux_session(raw: Option<&str>) -> LinuxSession {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) if s.eq_ignore_ascii_case("x11") => LinuxSession::X11,
        Some(s) if s.eq_ignore_ascii_case("wayland") => LinuxSession::Wayland,
        Some(_) => LinuxSession::Other,
        None => LinuxSession::Unknown,
    }
}

/// Probe the running Linux session type from the environment.
pub fn detect_linux_session() -> LinuxSession {
    if !cfg!(target_os = "linux") {
        return LinuxSession::Unknown;
    }
    classify_linux_session(std::env::var("XDG_SESSION_TYPE").ok().as_deref())
}

/// Short user-facing status when global hotkeys could not be registered.
pub const HOTKEYS_UNAVAILABLE_USER_MSG: &str = "Global hotkeys unavailable — use window controls";

/// Build a clear registration-failure log line (Linux-specific when applicable).
///
/// Includes session type on Linux, points at the X11 requirement of the current
/// stack, and tells the user to use in-window Start/Stop.
pub fn hotkey_registration_failure_log(err: &str, session: LinuxSession) -> String {
    let guidance = HOTKEYS_UNAVAILABLE_USER_MSG;
    if cfg!(target_os = "linux") {
        let session = session.as_str();
        format!(
            "Global hotkey registration failed (session={session}): {err}. \
             With the current stack, global hotkeys require X11 (see README / \
             research/linux-hotkey-paste-spec.md). Pure Wayland often cannot register. \
             {guidance}."
        )
    } else {
        format!("Global hotkey registration failed: {err}. {guidance}.")
    }
}

/// Outcome of registering dictation + command global shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyRegisterReport {
    pub dictation_ok: bool,
    pub command_ok: bool,
    pub errors: Vec<String>,
}

impl HotkeyRegisterReport {
    pub fn all_ok(&self) -> bool {
        self.dictation_ok && self.command_ok && self.errors.is_empty()
    }

    pub fn any_ok(&self) -> bool {
        self.dictation_ok || self.command_ok
    }

    /// Compact summary for logs / errors.
    pub fn summary(&self) -> String {
        if self.all_ok() {
            return "dictation+command registered".into();
        }
        let mut parts = Vec::new();
        if !self.dictation_ok {
            parts.push("dictation failed");
        }
        if !self.command_ok {
            parts.push("command failed");
        }
        if self.dictation_ok && !self.command_ok {
            parts.push("dictation ok");
        } else if self.command_ok && !self.dictation_ok {
            parts.push("command ok");
        }
        let detail = if self.errors.is_empty() {
            String::new()
        } else {
            format!(": {}", self.errors.join("; "))
        };
        format!("{}{}", parts.join(", "), detail)
    }
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
        assert_eq!(
            normalize_combo(" Ctrl + Shift + Space "),
            "Ctrl+Shift+Space"
        );
    }

    #[test]
    fn rejects_escape_alone_for_dictation_and_command() {
        for raw in ["Escape", "Esc", "ESCAPE", "esc"] {
            let d = validate_binding(raw, HotkeyRole::Dictation);
            assert!(d.is_err(), "expected reject for dictation {raw:?}");
            let msg = d.unwrap_err().to_string();
            assert!(
                msg.contains("Escape") && msg.contains("cancel"),
                "dictation error should mention Escape cancel: {msg}"
            );

            let c = validate_binding(raw, HotkeyRole::Command);
            assert!(c.is_err(), "expected reject for command {raw:?}");
            let msg = c.unwrap_err().to_string();
            assert!(
                msg.contains("Escape") && msg.contains("cancel"),
                "command error should mention Escape cancel: {msg}"
            );
        }
    }

    #[test]
    fn allows_escape_with_modifiers() {
        // Spec: Esc+modifiers not banned; only bare Escape is reserved.
        assert!(validate_binding("Ctrl+Esc", HotkeyRole::Dictation).is_ok());
        assert!(validate_binding("Ctrl+Shift+Escape", HotkeyRole::Command).is_ok());
    }

    #[test]
    fn escape_cancel_hotkey_parses() {
        let sc = parse_shortcut(ESCAPE_CANCEL_HOTKEY).unwrap();
        assert!(is_escape_alone(&sc));
    }

    #[test]
    fn classify_linux_session_values() {
        assert_eq!(classify_linux_session(None), LinuxSession::Unknown);
        assert_eq!(classify_linux_session(Some("")), LinuxSession::Unknown);
        assert_eq!(classify_linux_session(Some("   ")), LinuxSession::Unknown);
        assert_eq!(classify_linux_session(Some("x11")), LinuxSession::X11);
        assert_eq!(classify_linux_session(Some("X11")), LinuxSession::X11);
        assert_eq!(
            classify_linux_session(Some("wayland")),
            LinuxSession::Wayland
        );
        assert_eq!(
            classify_linux_session(Some("Wayland")),
            LinuxSession::Wayland
        );
        assert_eq!(classify_linux_session(Some("mir")), LinuxSession::Other);
        assert_eq!(classify_linux_session(Some("tty")), LinuxSession::Other);
    }

    #[test]
    fn hotkey_failure_log_mentions_window_controls_and_x11_on_linux() {
        let msg = hotkey_registration_failure_log(
            "other window systems are not supported",
            LinuxSession::Wayland,
        );
        assert!(
            msg.contains("use window controls") || msg.contains(HOTKEYS_UNAVAILABLE_USER_MSG),
            "expected window-controls guidance: {msg}"
        );
        if cfg!(target_os = "linux") {
            assert!(
                msg.contains("session=wayland"),
                "Linux log should include session type: {msg}"
            );
            assert!(
                msg.contains("X11"),
                "Linux log should mention X11 requirement: {msg}"
            );
        } else {
            assert!(
                msg.contains("Global hotkey registration failed"),
                "non-Linux path: {msg}"
            );
        }
    }

    #[test]
    fn hotkey_register_report_summary() {
        let ok = HotkeyRegisterReport {
            dictation_ok: true,
            command_ok: true,
            errors: vec![],
        };
        assert!(ok.all_ok());
        assert!(ok.any_ok());
        assert!(ok.summary().contains("registered"));

        let partial = HotkeyRegisterReport {
            dictation_ok: true,
            command_ok: false,
            errors: vec!["command grab denied".into()],
        };
        assert!(!partial.all_ok());
        assert!(partial.any_ok());
        let s = partial.summary();
        assert!(s.contains("command failed"), "{s}");
        assert!(s.contains("grab denied"), "{s}");

        let none = HotkeyRegisterReport {
            dictation_ok: false,
            command_ok: false,
            errors: vec!["no X11".into()],
        };
        assert!(!none.all_ok());
        assert!(!none.any_ok());
    }

    #[test]
    fn user_msg_is_stable() {
        assert!(HOTKEYS_UNAVAILABLE_USER_MSG.contains("window controls"));
        assert!(HOTKEYS_UNAVAILABLE_USER_MSG.contains("unavailable"));
    }
}
