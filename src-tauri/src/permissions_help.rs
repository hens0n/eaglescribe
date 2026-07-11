//! Failure-time permissions help classification (onboarding-permissions-spec §3.3).
//!
//! Maps known `last_error` strings to a stable hint code the UI uses to show the
//! **same** checklist guidance (mic / Accessibility / model) even after onboarding
//! was dismissed. Pure helpers — no I/O.

/// Kind of contextual permissions help to surface in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionsHelpKind {
    /// Empty capture / mic permission style failures.
    Microphone,
    /// Paste or inject simulation failures (macOS: Accessibility; all OS: clipboard).
    Accessibility,
    /// Missing or unloadable Whisper model.
    Model,
}

impl PermissionsHelpKind {
    /// Stable token for the frontend (`permissions_help` on the status snapshot).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::Accessibility => "accessibility",
            Self::Model => "model",
        }
    }

    pub fn from_str_token(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "microphone" | "mic" => Some(Self::Microphone),
            "accessibility" | "ax" | "paste" => Some(Self::Accessibility),
            "model" | "whisper" => Some(Self::Model),
            _ => None,
        }
    }
}

/// Classify a user-visible error string into a permissions-help kind.
///
/// Returns `None` for unrelated errors (busy, empty transcript polish, etc.).
/// Matching is case-insensitive and intentionally string-based so it works on
/// existing `last_error` messages without changing inject/audio strategy.
pub fn classify_permissions_help(error: &str) -> Option<PermissionsHelpKind> {
    let e = error.to_ascii_lowercase();
    if e.trim().is_empty() {
        return None;
    }

    // Model missing / load failure (existing STT messages already point at download).
    if e.contains("whisper model not found")
        || e.contains("failed to load whisper model")
        || e.contains("npm run model:download")
        || (e.contains("model path") && e.contains("not valid"))
    {
        return Some(PermissionsHelpKind::Model);
    }

    // Empty capture / mic permission style (audio stop + open failures).
    // Includes near-silent streams that still deliver samples (TCC deny → zeros).
    if e.contains("no audio captured")
        || e.contains("check microphone permissions")
        || e.contains("no default microphone")
        || e.contains("microphone never reported")
        || e.contains("microphone thread panicked")
        || e.contains("failed to enumerate microphones")
        || (e.contains("peak=")
            && e.contains("microphone")
            && (e.contains("permission") || e.contains("privacy")))
        || (e.contains("microphone")
            && (e.contains("permission")
                || e.contains("denied")
                || e.contains("not authorized")
                || e.contains("not authorised")))
    {
        return Some(PermissionsHelpKind::Microphone);
    }

    // Paste / inject simulation failure (INJ-03 / INJ-06). Clipboard still holds text.
    if e.contains("paste failed")
        || e.contains("paste simulation")
        || e.contains("inject failed")
        || e.contains("failed to schedule paste")
        || e.contains("paste timed out")
        || e.contains("failed to set clipboard")
        || e.contains("clipboard unavailable")
    {
        return Some(PermissionsHelpKind::Accessibility);
    }

    None
}

/// Classify optional `last_error` for the status snapshot field.
pub fn permissions_help_for_error(last_error: Option<&str>) -> Option<&'static str> {
    last_error
        .and_then(classify_permissions_help)
        .map(PermissionsHelpKind::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_empty_capture_as_microphone() {
        assert_eq!(
            classify_permissions_help("No audio captured — check microphone permissions"),
            Some(PermissionsHelpKind::Microphone)
        );
        assert_eq!(
            classify_permissions_help("No default microphone found"),
            Some(PermissionsHelpKind::Microphone)
        );
        assert_eq!(
            classify_permissions_help(
                "No audio captured — check microphone permissions (peak=0.0001). System Settings → Privacy & Security → Microphone → enable EagleScribe."
            ),
            Some(PermissionsHelpKind::Microphone)
        );
    }

    #[test]
    fn classifies_paste_failure_as_accessibility() {
        assert_eq!(
            classify_permissions_help(
                "Paste failed — transcript left on clipboard. Paste manually with Cmd/Ctrl+V."
            ),
            Some(PermissionsHelpKind::Accessibility)
        );
        assert_eq!(
            classify_permissions_help(
                "Inject failed — transcript left on clipboard. Paste manually with Cmd/Ctrl+V. (Clipboard unavailable: foo)"
            ),
            Some(PermissionsHelpKind::Accessibility)
        );
    }

    #[test]
    fn classifies_model_missing() {
        assert_eq!(
            classify_permissions_help(
                "Whisper model not found at /tmp/x\n\nDownload a ggml model, e.g.:\n  npm run model:download"
            ),
            Some(PermissionsHelpKind::Model)
        );
        assert_eq!(
            classify_permissions_help("Failed to load Whisper model: bad file"),
            Some(PermissionsHelpKind::Model)
        );
    }

    #[test]
    fn ignores_unrelated_errors() {
        assert_eq!(
            classify_permissions_help("Empty transcript (try speaking longer)"),
            None
        );
        assert_eq!(classify_permissions_help("Already recording"), None);
        assert_eq!(classify_permissions_help("Waiting on local LLM — please wait"), None);
        assert_eq!(classify_permissions_help(""), None);
    }

    #[test]
    fn snapshot_helper_returns_stable_tokens() {
        assert_eq!(
            permissions_help_for_error(Some(
                "No audio captured — check microphone permissions"
            )),
            Some("microphone")
        );
        assert_eq!(
            permissions_help_for_error(Some("Paste failed — transcript left on clipboard.")),
            Some("accessibility")
        );
        assert_eq!(permissions_help_for_error(None), None);
        assert_eq!(permissions_help_for_error(Some("Busy transcribing")), None);
    }

    #[test]
    fn kind_tokens_round_trip() {
        for kind in [
            PermissionsHelpKind::Microphone,
            PermissionsHelpKind::Accessibility,
            PermissionsHelpKind::Model,
        ] {
            assert_eq!(
                PermissionsHelpKind::from_str_token(kind.as_str()),
                Some(kind)
            );
        }
    }
}
