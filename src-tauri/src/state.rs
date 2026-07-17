use crate::audio::{self, RecordingSession};
use crate::dictionary::{self, DictEntry, Dictionary};
use crate::error::{AppError, AppResult};
use crate::history::{self, HistoryBook, HistoryEntry};
use crate::hotkey;
use crate::llm;
use crate::polish::{self, PolishMode};
use crate::recognition::{
    recognize_resampled, resample_capture, CapturedAudio, PreprocessingReport,
    RecognitionErrorKind, RecognitionOptions, SilenceTrimReport,
};
use crate::settings::{self, AppSettings, HotkeyMode};
use crate::snippets::{self, Snippet, SnippetBook};
use crate::stt::{self, WhisperEngine};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationStatus {
    Idle,
    Recording,
    Transcribing,
    /// Command Mode: waiting on local LLM rewrite (after STT).
    WaitingLlm,
    Error,
}

impl DictationStatus {
    /// True while a background STT/LLM job should block new sessions.
    pub fn is_busy(self) -> bool {
        matches!(self, Self::Transcribing | Self::WaitingLlm)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionKind {
    Dictation,
    Command,
}

pub struct AppState {
    inner: Mutex<InnerState>,
    /// While true, Command Mode hotkey Released events are ignored.
    /// Prevents synthetic Cmd/Ctrl+C (selection capture) from ending the session.
    suppress_command_release: AtomicBool,
    /// After cancel mid-hold (Escape / UI Cancel): ignore dictation & command
    /// hotkey Released so a still-held chord does not stop-into-transcribe.
    /// Cleared on the next Released (consumed) so a later press starts cleanly.
    suppress_hotkey_release_after_cancel: AtomicBool,
}

struct InnerState {
    status: DictationStatus,
    session_kind: SessionKind,
    /// Selected text captured at the start of Command Mode (may be empty).
    command_selection: Option<String>,
    /// Ignore command-hotkey releases until this time (debounce after arming).
    command_ignore_release_until: Option<Instant>,
    model_path: PathBuf,
    settings_path: PathBuf,
    settings: AppSettings,
    dictionary_path: PathBuf,
    dictionary: Dictionary,
    snippets_path: PathBuf,
    snippets: SnippetBook,
    history_path: PathBuf,
    history: HistoryBook,
    /// Shared so STT can run without holding the state mutex (avoids main-thread deadlock).
    engine: Option<Arc<WhisperEngine>>,
    session: Option<RecordingSession>,
    polish_mode: PolishMode,
    last_transcript: Option<String>,
    last_raw_transcript: Option<String>,
    last_error: Option<String>,
    /// Open-time mic label from the most recent recording start.
    last_input_device_label: Option<String>,
    /// Backend-computed fallback notice when preferred mic was unavailable (UI displays as-is).
    last_mic_fallback_notice: Option<String>,
    /// True only when both dictation + command global shortcuts registered successfully.
    /// Derived from the per-role flags; kept for older UI checks.
    global_hotkeys_ok: bool,
    /// OS registration for the dictation chord (independent of command).
    dictation_hotkey_ok: bool,
    /// OS registration for the Command Mode chord (independent of dictation).
    command_hotkey_ok: bool,
    log: Vec<String>,
}

impl AppState {
    pub fn new(model_path: PathBuf) -> Self {
        let settings_path = settings::default_settings_path();
        let settings = AppSettings::load_or_default(&settings_path);

        let dictionary_path = dictionary::default_dictionary_path();
        let dictionary = Dictionary::load_or_default(&dictionary_path);
        let entry_count = dictionary.entries.len();

        let snippets_path = snippets::default_snippets_path();
        let snippets = SnippetBook::load_or_default(&snippets_path);
        let snippet_count = snippets.snippets.len();

        let history_path = history::default_history_path();
        let history = HistoryBook::load_or_default(&history_path);
        let history_count = history.entries.len();
        let history_enabled = settings.history_enabled;
        let clipboard_restore = settings.clipboard_restore;
        let silence_trim = settings.silence_trim;

        let hotkey_mode = settings.hotkey_mode;
        let command_hotkey = settings.command_hotkey.clone();
        let dictation_hotkey = settings.dictation_hotkey.clone();

        Self {
            suppress_command_release: AtomicBool::new(false),
            suppress_hotkey_release_after_cancel: AtomicBool::new(false),
            inner: Mutex::new(InnerState {
                status: DictationStatus::Idle,
                session_kind: SessionKind::Dictation,
                command_selection: None,
                command_ignore_release_until: None,
                model_path,
                settings_path: settings_path.clone(),
                settings,
                dictionary_path: dictionary_path.clone(),
                dictionary,
                snippets_path: snippets_path.clone(),
                snippets,
                history_path: history_path.clone(),
                history,
                engine: None,
                session: None,
                polish_mode: PolishMode::Smart,
                last_transcript: None,
                last_raw_transcript: None,
                last_error: None,
                last_input_device_label: None,
                last_mic_fallback_notice: None,
                // Until setup attempts OS registration, do not claim shortcuts are active.
                global_hotkeys_ok: false,
                dictation_hotkey_ok: false,
                command_hotkey_ok: false,
                log: {
                    let mut log = vec!["EagleScribe ready.".into()];
                    log.push(format!(
                        "Hotkey mode: {} ({})",
                        hotkey_mode.as_str(),
                        hotkey_mode.label()
                    ));
                    log.push(format!("Dictation hotkey (configured): {dictation_hotkey}"));
                    log.push(format!(
                        "Command Mode (configured): {command_hotkey} (select text, hold, speak instruction)"
                    ));
                    log.push(format!(
                        "Dictionary: {} ({} entries)",
                        dictionary_path.display(),
                        entry_count
                    ));
                    log.push(format!(
                        "Snippets: {} ({} cues)",
                        snippets_path.display(),
                        snippet_count
                    ));
                    log.push(format!(
                        "History: {} ({} entries, {})",
                        history_path.display(),
                        history_count,
                        if history_enabled { "on" } else { "off" }
                    ));
                    log.push(format!(
                        "Clipboard restore after paste: {}",
                        if clipboard_restore { "on" } else { "off" }
                    ));
                    log.push(format!(
                        "Silence trim (leading/trailing): {}",
                        if silence_trim { "on" } else { "off" }
                    ));
                    log
                },
            }),
        }
    }

    /// True when a Command Mode Released event should be ignored (arming / debounce).
    pub fn should_ignore_command_release(&self) -> bool {
        if self.suppress_command_release.load(Ordering::SeqCst) {
            return true;
        }
        if self.consume_hotkey_release_suppress() {
            return true;
        }
        let g = self.inner.lock();
        g.command_ignore_release_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false)
    }

    /// True while a dictation or command session is actively capturing audio.
    pub fn is_recording(&self) -> bool {
        self.inner.lock().status == DictationStatus::Recording
    }

    /// After cancel mid-hold: next hotkey Released should be ignored (once).
    /// Returns true if this release was suppressed (caller must not stop/transcribe).
    pub fn consume_hotkey_release_suppress(&self) -> bool {
        self.suppress_hotkey_release_after_cancel
            .swap(false, Ordering::SeqCst)
    }

    /// Whether a post-cancel release suppress is still pending (does not clear).
    #[cfg(test)]
    pub fn has_hotkey_release_suppress(&self) -> bool {
        self.suppress_hotkey_release_after_cancel
            .load(Ordering::SeqCst)
    }

    pub fn snapshot(&self) -> StatusSnapshot {
        let g = self.inner.lock();
        StatusSnapshot {
            status: g.status,
            model_path: g.model_path.display().to_string(),
            model_loaded: g.engine.is_some(),
            polish_mode: g.polish_mode,
            hotkey_mode: g.settings.hotkey_mode,
            dictation_hotkey: g.settings.dictation_hotkey.clone(),
            command_hotkey: g.settings.command_hotkey.clone(),
            llm_base_url: g.settings.llm_base_url.clone(),
            llm_model: g.settings.llm_model.clone(),
            dictionary_path: g.dictionary_path.display().to_string(),
            dictionary: g.dictionary.list(),
            snippets_path: g.snippets_path.display().to_string(),
            snippets: g.snippets.list(),
            history_path: g.history_path.display().to_string(),
            history_enabled: g.settings.history_enabled,
            history_max: g.settings.history_max,
            history: g.history.list_newest_first(),
            clipboard_restore: g.settings.clipboard_restore,
            silence_trim: g.settings.silence_trim,
            menu_bar_only: g.settings.menu_bar_only,
            // True only on macOS builds — UI hides the control elsewhere.
            menu_bar_only_available: cfg!(target_os = "macos"),
            // None / empty = system default (UI shows "System default").
            input_device_name: audio::normalize_input_device_name(
                g.settings.input_device_name.as_deref(),
            ),
            last_input_device_label: g.last_input_device_label.clone(),
            last_mic_fallback_notice: g.last_mic_fallback_notice.clone(),
            last_transcript: g.last_transcript.clone(),
            last_raw_transcript: g.last_raw_transcript.clone(),
            last_error: g.last_error.clone(),
            // Failure-time permissions hint (ignores onboarding_dismissed). UI maps code → copy.
            permissions_help: crate::permissions_help::permissions_help_for_error(
                g.last_error.as_deref(),
            )
            .map(str::to_string),
            log: g.log.clone(),
            session_kind: match g.session_kind {
                SessionKind::Dictation => "dictation".into(),
                SessionKind::Command => "command".into(),
            },
            // Compile-time Whisper backend (metal/cuda/vulkan/cpu) — read-only UI.
            stt_accel: stt::stt_acceleration().into(),
            // Soft Settings hint only; never blocks load/dictation.
            show_metal_rebuild_hint: stt::show_metal_rebuild_hint(),
            // OS global-shortcut registration result (never assume true before setup).
            global_hotkeys_ok: g.global_hotkeys_ok,
            dictation_hotkey_ok: g.dictation_hotkey_ok,
            command_hotkey_ok: g.command_hotkey_ok,
            // Linux `$XDG_SESSION_TYPE` probe; null/omitted meaning on non-Linux via "unknown".
            linux_session: hotkey::detect_linux_session().as_str().to_string(),
            onboarding_dismissed: g.settings.onboarding_dismissed,
            // Compile-time host OS for first-run / permissions copy (macos | linux | other).
            host_os: host_os_label().into(),
        }
    }

    /// Persist first-run checklist dismiss (Settings re-open still works).
    pub fn set_onboarding_dismissed(&self, dismissed: bool) -> AppResult<()> {
        let mut g = self.inner.lock();
        g.settings.onboarding_dismissed = dismissed;
        g.settings.save(&g.settings_path)?;
        g.log.push(format!(
            "Setup checklist: {}",
            if dismissed {
                "dismissed (re-open from Settings anytime)"
            } else {
                "will show on next launch"
            }
        ));
        Ok(())
    }

    /// Record the outcome of OS global-shortcut registration (startup or rebind).
    ///
    /// Prefer [`set_hotkey_registration`] when per-role results are known.
    pub fn set_global_hotkeys_ok(&self, ok: bool) {
        let mut g = self.inner.lock();
        g.global_hotkeys_ok = ok;
        // Keep per-role flags aligned when only a combined result is available.
        g.dictation_hotkey_ok = ok;
        g.command_hotkey_ok = ok;
    }

    /// Record per-role OS registration results (startup or rebind).
    pub fn set_hotkey_registration(&self, dictation_ok: bool, command_ok: bool) {
        let mut g = self.inner.lock();
        g.dictation_hotkey_ok = dictation_ok;
        g.command_hotkey_ok = command_ok;
        g.global_hotkeys_ok = dictation_ok && command_ok;
    }

    /// Record open-time mic info; on structured fallback, push a clear log + snapshot notice.
    ///
    /// Uses [`audio::MicOpenInfo`] flags — does not re-parse free-form device labels.
    fn note_recording_mic(g: &mut InnerState, info: &audio::MicOpenInfo) {
        g.last_input_device_label = Some(info.device_label.clone());
        g.last_mic_fallback_notice = info.fallback_notice();
        if let Some(notice) = &g.last_mic_fallback_notice {
            eprintln!("[eaglescribe] {notice}");
            g.log.push(notice.clone());
        }
    }

    pub fn hotkey_mode(&self) -> HotkeyMode {
        self.inner.lock().settings.hotkey_mode
    }

    pub fn dictation_hotkey(&self) -> String {
        self.inner.lock().settings.dictation_hotkey.clone()
    }

    pub fn command_hotkey(&self) -> String {
        self.inner.lock().settings.command_hotkey.clone()
    }

    pub fn push_log(&self, msg: impl Into<String>) {
        let mut g = self.inner.lock();
        let msg = msg.into();
        eprintln!("[eaglescribe] {msg}");
        g.log.push(msg);
        if g.log.len() > 100 {
            let drain = g.log.len() - 100;
            g.log.drain(0..drain);
        }
    }

    pub fn set_model_path(&self, path: PathBuf) {
        let mut g = self.inner.lock();
        g.model_path = path;
        g.engine = None; // drop Arc; reload on next ensure_engine
        g.log
            .push("Model path updated; will reload on next dictation.".into());
    }

    pub fn set_polish_mode(&self, mode: PolishMode) {
        let mut g = self.inner.lock();
        g.polish_mode = mode;
        g.log.push(format!("Polish mode: {mode:?}"));
    }

    pub fn set_hotkey_mode(&self, mode: HotkeyMode) -> AppResult<()> {
        let mut g = self.inner.lock();
        g.settings.hotkey_mode = mode;
        g.settings.save(&g.settings_path)?;
        g.log
            .push(format!("Hotkey mode: {} ({})", mode.as_str(), mode.label()));
        Ok(())
    }

    /// Persist validated hotkey combos (caller re-registers OS shortcuts).
    pub fn set_hotkey_bindings(&self, dictation: &str, command: &str) -> AppResult<()> {
        let (dictation, command) = hotkey::validate_pair(dictation, command)?;
        let mut g = self.inner.lock();
        g.settings.dictation_hotkey = dictation.clone();
        g.settings.command_hotkey = command.clone();
        g.settings.save(&g.settings_path)?;
        g.log.push(format!(
            "Dictation hotkey: {dictation} · Command: {command}"
        ));
        Ok(())
    }

    pub fn set_history_enabled(&self, enabled: bool) -> AppResult<()> {
        let mut g = self.inner.lock();
        g.settings.history_enabled = enabled;
        g.settings.save(&g.settings_path)?;
        g.log.push(format!(
            "Transcript history: {}",
            if enabled { "on" } else { "off" }
        ));
        Ok(())
    }

    pub fn set_clipboard_restore(&self, enabled: bool) -> AppResult<()> {
        let mut g = self.inner.lock();
        g.settings.clipboard_restore = enabled;
        g.settings.save(&g.settings_path)?;
        g.log.push(format!(
            "Clipboard restore after paste: {}",
            if enabled { "on" } else { "off" }
        ));
        Ok(())
    }

    /// Persist leading/trailing silence trim (applies on next completed take).
    pub fn set_silence_trim(&self, enabled: bool) -> AppResult<()> {
        let mut g = self.inner.lock();
        g.settings.silence_trim = enabled;
        g.settings.save(&g.settings_path)?;
        g.log.push(format!(
            "Silence trim (leading/trailing): {}",
            if enabled { "on" } else { "off" }
        ));
        Ok(())
    }

    /// Persist macOS menu-bar-only (hide Dock). Takes effect on next launch.
    pub fn set_menu_bar_only(&self, enabled: bool) -> AppResult<()> {
        let mut g = self.inner.lock();
        g.settings.menu_bar_only = enabled;
        g.settings.save(&g.settings_path)?;
        g.log.push(format!(
            "Menu bar only (hide Dock): {} — takes effect next launch",
            if enabled { "on" } else { "off" }
        ));
        Ok(())
    }

    /// Current menu-bar-only preference (for launch-time activation policy).
    pub fn menu_bar_only(&self) -> bool {
        self.inner.lock().settings.menu_bar_only
    }

    /// Persist preferred microphone (`None` / empty = system default).
    pub fn set_input_device(&self, name: Option<&str>) -> AppResult<()> {
        let normalized = audio::normalize_input_device_name(name);
        let mut g = self.inner.lock();
        g.settings.input_device_name = normalized.clone();
        g.settings.save(&g.settings_path)?;
        let msg = match &normalized {
            Some(n) => format!("Microphone: {n}"),
            None => "Microphone: system default".into(),
        };
        g.log.push(msg);
        Ok(())
    }

    /// Preferred mic name for the next recording (`None` = system default).
    fn preferred_input_device(&self) -> Option<String> {
        let g = self.inner.lock();
        audio::normalize_input_device_name(g.settings.input_device_name.as_deref())
    }

    pub fn clear_history(&self) -> AppResult<()> {
        let mut g = self.inner.lock();
        g.history.clear();
        g.history.save(&g.history_path)?;
        g.log.push("Transcript history cleared.".into());
        Ok(())
    }

    pub fn set_llm_settings(&self, base_url: &str, model: &str, api_key: &str) -> AppResult<()> {
        let mut g = self.inner.lock();
        g.settings.llm_base_url = base_url.trim().to_string();
        g.settings.llm_model = model.trim().to_string();
        g.settings.llm_api_key = api_key.to_string();
        g.settings.save(&g.settings_path)?;
        let msg = format!(
            "LLM: {} model={}",
            g.settings.llm_base_url, g.settings.llm_model
        );
        g.log.push(msg);
        Ok(())
    }

    pub fn dictionary_add(&self, from: &str, to: &str) -> AppResult<()> {
        let mut g = self.inner.lock();
        g.dictionary.upsert(from, to)?;
        g.dictionary.save(&g.dictionary_path)?;
        g.log
            .push(format!("Dictionary: {:?} → {:?}", from.trim(), to.trim()));
        Ok(())
    }

    pub fn dictionary_remove(&self, from: &str) -> AppResult<()> {
        let mut g = self.inner.lock();
        if !g.dictionary.remove(from) {
            return Err(AppError::from(format!(
                "No dictionary entry for {:?}",
                from.trim()
            )));
        }
        g.dictionary.save(&g.dictionary_path)?;
        g.log.push(format!("Dictionary removed: {:?}", from.trim()));
        Ok(())
    }

    pub fn snippet_add(&self, cue: &str, expansion: &str) -> AppResult<()> {
        let mut g = self.inner.lock();
        g.snippets.upsert(cue, expansion)?;
        g.snippets.save(&g.snippets_path)?;
        g.log.push(format!(
            "Snippet: {:?} → {} chars",
            cue.trim(),
            expansion.trim().chars().count()
        ));
        Ok(())
    }

    pub fn snippet_remove(&self, cue: &str) -> AppResult<()> {
        let mut g = self.inner.lock();
        if !g.snippets.remove(cue) {
            return Err(AppError::from(format!(
                "No snippet for cue {:?}",
                cue.trim()
            )));
        }
        g.snippets.save(&g.snippets_path)?;
        g.log.push(format!("Snippet removed: {:?}", cue.trim()));
        Ok(())
    }

    pub fn ensure_engine(&self) -> AppResult<()> {
        let mut g = self.inner.lock();
        if g.engine.is_some() {
            return Ok(());
        }
        let path = g.model_path.clone();
        g.log
            .push(format!("Loading Whisper model: {}", path.display()));
        drop(g);

        let engine = WhisperEngine::load(&path)?;

        let mut g = self.inner.lock();
        g.engine = Some(Arc::new(engine));
        g.log.push("Whisper model loaded.".into());
        Ok(())
    }

    fn reject_if_unavailable(status: DictationStatus) -> AppResult<()> {
        if status == DictationStatus::Recording {
            return Err(AppError::from("Already recording"));
        }
        if status.is_busy() {
            return Err(AppError::from(match status {
                DictationStatus::WaitingLlm => "Waiting on local LLM — please wait",
                _ => "Busy transcribing — please wait",
            }));
        }
        Ok(())
    }

    pub fn start_recording(&self) -> AppResult<()> {
        {
            let g = self.inner.lock();
            Self::reject_if_unavailable(g.status)?;
        }

        // Open the mic without holding `inner` (enumeration + up to ~500ms sample-rate wait).
        let preferred = self.preferred_input_device();
        let session = RecordingSession::start(preferred.as_deref())?;
        let open_info = audio::MicOpenInfo {
            device_label: session.device_label.clone(),
            preferred_unavailable: session.preferred_unavailable.clone(),
        };

        let mut g = self.inner.lock();
        // Another session may have started while we opened the mic.
        if g.status == DictationStatus::Recording || g.status.is_busy() {
            drop(session);
            return Err(AppError::from("Busy — cannot start recording"));
        }
        g.session = Some(session);
        g.session_kind = SessionKind::Dictation;
        g.command_selection = None;
        g.status = DictationStatus::Recording;
        g.last_error = None;
        Self::note_recording_mic(&mut g, &open_info);
        g.log.push(format!(
            "Recording… mic={} (release hotkey or use Stop to finish)",
            open_info.device_label
        ));
        // New intentional session: do not inherit a post-cancel release suppress
        // from a prior cancel that happened without a held chord.
        self.suppress_hotkey_release_after_cancel
            .store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Command Mode: capture selection, then record a spoken instruction.
    ///
    /// Selection capture synthesizes Cmd/Ctrl+C. That can spuriously fire
    /// Released on hotkeys that include the C key, so we suppress command
    /// releases while arming and for a short debounce window afterward.
    pub fn start_command_recording<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<()> {
        {
            let g = self.inner.lock();
            Self::reject_if_unavailable(g.status)?;
        }

        self.suppress_command_release.store(true, Ordering::SeqCst);

        let preferred = self.preferred_input_device();
        let selection = (|| {
            self.push_log("Command Mode: capturing selection (Cmd/Ctrl+C)…");
            let selection = crate::inject::capture_selection(app).unwrap_or_default();
            if selection.trim().is_empty() {
                self.push_log("Command Mode: no selection — will generate at cursor.");
            } else {
                self.push_log(format!(
                    "Command Mode: {} chars selected",
                    selection.chars().count()
                ));
            }

            let session = RecordingSession::start(preferred.as_deref())?;
            let open_info = audio::MicOpenInfo {
                device_label: session.device_label.clone(),
                preferred_unavailable: session.preferred_unavailable.clone(),
            };
            let mut g = self.inner.lock();
            // Another session may have started while we captured selection.
            if g.status == DictationStatus::Recording || g.status.is_busy() {
                return Err(AppError::from("Busy — cannot start Command Mode"));
            }
            g.session = Some(session);
            g.session_kind = SessionKind::Command;
            g.command_selection = Some(selection);
            // Ignore hotkey releases for a bit after arming (synthetic key noise).
            g.command_ignore_release_until = Some(Instant::now() + Duration::from_millis(400));
            g.status = DictationStatus::Recording;
            g.last_error = None;
            Self::note_recording_mic(&mut g, &open_info);
            g.log.push(format!(
                "Command Mode recording… mic={} · speak your instruction",
                open_info.device_label
            ));
            // Fresh session — drop any leftover post-cancel release suppress.
            self.suppress_hotkey_release_after_cancel
                .store(false, Ordering::SeqCst);
            Ok(())
        })();

        self.suppress_command_release.store(false, Ordering::SeqCst);
        selection
    }

    /// Stop mic, transcribe, polish, dictionary, snippets, inject.
    fn set_status_emit<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        status: DictationStatus,
        log: Option<String>,
    ) {
        {
            let mut g = self.inner.lock();
            g.status = status;
            if let Some(msg) = log {
                g.log.push(msg);
                if g.log.len() > 100 {
                    let drain = g.log.len() - 100;
                    g.log.drain(0..drain);
                }
            }
        }
        // Never hold the state lock across emit (UI / main-thread handlers may re-enter).
        let _ = app.emit("dictation-status", self.snapshot());
    }

    fn fail_status<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        status: DictationStatus,
        err: impl Into<String>,
    ) {
        let err = err.into();
        {
            let mut g = self.inner.lock();
            g.status = status;
            g.last_error = Some(err);
        }
        let _ = app.emit("dictation-status", self.snapshot());
    }

    fn log_preprocessing(&self, report: &PreprocessingReport) {
        match report.silence_trim {
            SilenceTrimReport::Disabled => {}
            SilenceTrimReport::Applied {
                original_ms,
                trimmed_ms,
                head_ms,
                tail_ms,
                threshold,
            } => self.push_log(format!(
                "Silence trim: {:.1}s → {:.1}s (removed head {head_ms}ms, tail {tail_ms}ms, threshold={threshold:.4})",
                original_ms as f32 / 1000.0,
                trimmed_ms as f32 / 1000.0,
            )),
            SilenceTrimReport::KeptFullBuffer {
                original_ms,
                remaining_ms,
                threshold,
            } => self.push_log(format!(
                "Silence trim found no speech frames ({original_ms}ms → {remaining_ms}ms, peak={peak:.4}, threshold={threshold:.4}) — keeping full buffer for STT",
                peak = report.peak,
            )),
        }
        self.push_log(format!(
            "STT pad: +{}ms trailing silence for decoder",
            report.decoder_tail_padding_ms
        ));
    }

    /// For Command Mode, runs local LLM rewrite instead of normal paste pipeline.
    pub fn stop_and_transcribe<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<String> {
        // Claim the session and mark busy under one lock so back-to-back
        // hotkeys cannot start a second worker while STT is still running.
        let (session, kind, command_selection) = {
            let mut g = self.inner.lock();
            if g.status != DictationStatus::Recording {
                return Err(AppError::from("Not recording"));
            }
            let session = match g.session.take() {
                Some(s) => s,
                None => {
                    g.status = DictationStatus::Error;
                    g.last_error = Some("Missing recording session".into());
                    drop(g);
                    let _ = app.emit("dictation-status", self.snapshot());
                    return Err(AppError::from("Missing recording session"));
                }
            };
            let kind = g.session_kind;
            let sel = g.command_selection.take();
            g.session_kind = SessionKind::Dictation;
            g.status = DictationStatus::Transcribing;
            g.log.push("Transcribing on-device…".into());
            (session, kind, sel)
        };
        let _ = app.emit("dictation-status", self.snapshot());

        if let Err(e) = self.ensure_engine() {
            self.fail_status(app, DictationStatus::Error, e.to_string());
            return Err(e);
        }

        // Post-roll: the hold hotkey Released (or Stop) already fired, but the
        // mic session is still open until `session.stop()`. Keep capturing so
        // words at the end of a second sentence are not lost when the chord is
        // lifted a beat early. Offline STT on full audio is fine; history
        // truncations matched incomplete capture duration, not Whisper.
        let post_roll = crate::audio::RECORDING_POST_ROLL_MS;
        if post_roll > 0 {
            self.push_log(format!(
                "Post-roll {post_roll}ms (keep mic open after release)…"
            ));
            std::thread::sleep(std::time::Duration::from_millis(post_roll));
        }

        let capture = match CapturedAudio::from_capture(session.stop()) {
            Ok(capture) => capture,
            Err(error) => {
                self.fail_status(app, DictationStatus::Error, error.to_string());
                return Err(AppError::from(error.to_string()));
            }
        };

        let resampled = match resample_capture(capture) {
            Ok(resampled) => resampled,
            Err(error) => {
                self.fail_status(app, DictationStatus::Error, error.to_string());
                return Err(AppError::from(error.to_string()));
            }
        };
        // Keep the ordinary-dictation dogfood dump outside the reusable path.
        // Tuning callers use `recognize_raw` directly and never persist audio.
        let duration_s = resampled.samples().len() as f32 / 16_000.0;
        let peak = crate::audio::peak_abs(resampled.samples());
        self.push_log(format!(
            "Captured {duration_s:.1}s audio ({} samples @ 16 kHz, peak={peak:.4}, device rate={rate})",
            resampled.samples().len(),
            rate = resampled.input_sample_rate(),
        ));
        // Overwrite last-take dump so we can re-run STT offline if text looks cut off.
        let capture_path = crate::audio::default_last_capture_path();
        match crate::audio::write_wav_16k_mono(&capture_path, resampled.samples()) {
            Ok(()) => self.push_log(format!(
                "Saved last capture ({duration_s:.1}s) → {}",
                capture_path.display()
            )),
            Err(e) => self.push_log(format!("Could not save last capture wav: {e}")),
        };

        let silence_trim = {
            let g = self.inner.lock();
            g.settings.silence_trim
        };

        // Clone Arc so Whisper runs without holding the state mutex.
        // Holding the mutex during STT deadlocks with main-thread paste/hotkeys.
        let engine = {
            let g = self.inner.lock();
            g.engine
                .clone()
                .ok_or_else(|| AppError::from("Engine not loaded"))?
        };

        let recognition = recognize_resampled(
            resampled,
            RecognitionOptions { silence_trim },
            engine.as_ref(),
        );
        if let Err(error) = &recognition {
            if let Some(preprocessing) = error.preprocessing() {
                self.log_preprocessing(preprocessing);
            }
        }
        let recognition = match recognition {
            Ok(recognition) => recognition,
            Err(error) if error.kind() == RecognitionErrorKind::EmptyTranscript => {
                {
                    let mut g = self.inner.lock();
                    g.status = DictationStatus::Idle;
                    g.last_error = Some("Empty transcript (try speaking longer)".into());
                    g.log.push("Empty transcript.".into());
                }
                let _ = app.emit("dictation-status", self.snapshot());
                return Err(AppError::from(error.to_string()));
            }
            Err(error) => {
                if error.kind() == RecognitionErrorKind::SilentAudio {
                    self.push_log(error.to_string());
                }
                self.fail_status(app, DictationStatus::Error, error.to_string());
                return Err(AppError::from(error.to_string()));
            }
        };

        self.log_preprocessing(&recognition.preprocessing);

        let raw = recognition.transcript;

        if kind == SessionKind::Command {
            return self.finish_command_mode(app, &raw, command_selection.unwrap_or_default());
        }

        let (mode, dictionary, snippets) = {
            let g = self.inner.lock();
            (g.polish_mode, g.dictionary.clone(), g.snippets.clone())
        };

        let polished = polish::polish(&raw, mode);
        if polished.changed {
            self.push_log(format!(
                "Polished: {} → {}",
                truncate(&polished.raw, 40),
                truncate(&polished.polished, 40)
            ));
        } else {
            self.push_log("Polish: no changes (or verbatim mode)");
        }

        let after_dict = dictionary.apply(&polished.polished);
        if after_dict != polished.polished {
            self.push_log(format!(
                "Dictionary: {} → {}",
                truncate(&polished.polished, 40),
                truncate(&after_dict, 40)
            ));
        }

        let (after_snip, snip_changed) = snippets.apply(&after_dict);
        if snip_changed {
            self.push_log(format!(
                "Snippet: {} → {}",
                truncate(&after_dict, 40),
                truncate(&after_snip, 40)
            ));
        }

        let text = after_snip;
        if text.is_empty() {
            {
                let mut g = self.inner.lock();
                g.status = DictationStatus::Idle;
                g.last_raw_transcript = Some(polished.raw);
                g.last_error = Some("Transcript empty after polish".into());
            }
            let _ = app.emit("dictation-status", self.snapshot());
            return Err(AppError::from("Transcript empty after polish"));
        }

        self.finish_inject(app, &polished.raw, &text, SessionKind::Dictation)
    }

    fn finish_command_mode<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        instruction_raw: &str,
        selection: String,
    ) -> AppResult<String> {
        let mode = self.inner.lock().polish_mode;
        let instruction = polish::polish(instruction_raw, mode).polished;
        self.push_log(format!(
            "Command instruction: {}",
            truncate(&instruction, 80)
        ));

        let llm = self.inner.lock().settings.llm_config();
        let (system, user) = llm::build_rewrite_prompt(&instruction, &selection);

        self.set_status_emit(
            app,
            DictationStatus::WaitingLlm,
            Some(format!(
                "Command Mode: waiting on local LLM {} …",
                llm.model
            )),
        );

        // HTTP call — must not hold state lock (and must not block main thread).
        let rewritten = match llm::complete(&llm, &system, &user) {
            Ok(t) => t,
            Err(e) => {
                {
                    let mut g = self.inner.lock();
                    g.status = DictationStatus::Error;
                    g.last_error = Some(e.to_string());
                    g.last_raw_transcript = Some(instruction_raw.to_string());
                }
                let _ = app.emit("dictation-status", self.snapshot());
                return Err(e);
            }
        };

        self.push_log(format!("Command result: {}", truncate(&rewritten, 80)));
        self.finish_inject(app, instruction_raw, &rewritten, SessionKind::Command)
    }

    fn finish_inject<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        raw: &str,
        text: &str,
        kind: SessionKind,
    ) -> AppResult<String> {
        // Read prefs before paste; never hold `inner` across main-thread inject.
        let restore_clipboard = {
            let g = self.inner.lock();
            g.settings.clipboard_restore
        };

        match crate::inject::inject_text(app, text, restore_clipboard) {
            Ok(result) => {
                {
                    let mut g = self.inner.lock();
                    g.last_raw_transcript = Some(raw.to_string());
                    g.last_transcript = Some(result.text.clone());
                    g.status = DictationStatus::Idle;
                    if result.pasted {
                        g.last_error = None;
                        g.log
                            .push(format!("Injected: {}", truncate(&result.text, 80)));
                        if result.restored {
                            g.log
                                .push("Restored previous clipboard after paste.".into());
                        } else if result.restore_failed {
                            g.log.push(
                                "Clipboard restore failed; transcript left on clipboard.".into(),
                            );
                        }
                    } else {
                        // INJ-03: paste failed — transcript stays on clipboard; surface in UI
                        // (error banner), not log-only / eprintln-only.
                        g.last_error = Some(
                            "Paste failed — transcript left on clipboard. Paste manually with Cmd/Ctrl+V."
                                .into(),
                        );
                        g.log.push(format!(
                            "Copied (paste manually with Cmd/Ctrl+V): {}",
                            truncate(&result.text, 80)
                        ));
                    }
                    Self::maybe_record_history(&mut g, kind, &result.text, Some(raw));
                }
                let _ = app.emit("dictation-status", self.snapshot());
                Ok(result.text)
            }
            Err(e) => {
                let _ = crate::inject::copy_to_clipboard(text);
                {
                    let mut g = self.inner.lock();
                    g.last_raw_transcript = Some(raw.to_string());
                    g.last_transcript = Some(text.to_string());
                    g.status = DictationStatus::Idle;
                    // Keep paste/clipboard wording so failure-time help classifies as Accessibility.
                    g.last_error = Some(format!(
                        "Inject failed — transcript left on clipboard. Paste manually with Cmd/Ctrl+V. ({e})"
                    ));
                    g.log
                        .push(format!("Transcript on clipboard; inject failed: {e}"));
                    // Still record — user got the text on the clipboard.
                    Self::maybe_record_history(&mut g, kind, text, Some(raw));
                }
                let _ = app.emit("dictation-status", self.snapshot());
                Ok(text.to_string())
            }
        }
    }

    fn maybe_record_history(g: &mut InnerState, kind: SessionKind, text: &str, raw: Option<&str>) {
        if !g.settings.history_enabled {
            return;
        }
        let max = g.settings.history_max.max(1);
        let kind_str = match kind {
            SessionKind::Dictation => "dictation",
            SessionKind::Command => "command",
        };
        g.history.push(kind_str, text, raw, max);
        if let Err(e) = g.history.save(&g.history_path) {
            g.log
                .push(format!("History save failed (in-memory only): {e}"));
        }
    }

    pub fn cancel_recording(&self) -> AppResult<()> {
        {
            let mut g = self.inner.lock();
            if g.status != DictationStatus::Recording {
                return Err(AppError::from("Not recording"));
            }
            let _ = g.session.take();
            g.command_selection = None;
            // Keep ignore window clear; hold-safety uses suppress_hotkey_release_after_cancel.
            g.command_ignore_release_until = None;
            g.session_kind = SessionKind::Dictation;
            g.status = DictationStatus::Idle;
        }
        // Ignore leftover hotkey Released from a still-held chord (dictation hold
        // or Command Mode). Cleared on the next Released *or* when a new session starts.
        self.suppress_hotkey_release_after_cancel
            .store(true, Ordering::SeqCst);
        self.push_log("Recording cancelled.");
        Ok(())
    }

    /// Test helper: mark status as recording without opening the mic.
    #[cfg(test)]
    pub fn force_recording_for_test(&self) {
        let mut g = self.inner.lock();
        g.status = DictationStatus::Recording;
        g.session_kind = SessionKind::Dictation;
        g.session = None;
        g.command_selection = None;
        // Match start_recording: a new session clears stale post-cancel suppress.
        self.suppress_hotkey_release_after_cancel
            .store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> AppState {
        AppState::new(PathBuf::from("/tmp/eaglescribe-test-model.bin"))
    }

    #[test]
    fn cancel_recording_sets_idle_and_logs() {
        let state = test_state();
        state.force_recording_for_test();
        state.cancel_recording().unwrap();
        let snap = state.snapshot();
        assert_eq!(snap.status, DictationStatus::Idle);
        assert!(
            snap.log.iter().any(|l| l.contains("cancelled")),
            "expected cancel log, got {:?}",
            snap.log
        );
    }

    #[test]
    fn cancel_when_not_recording_is_error() {
        let state = test_state();
        let err = state.cancel_recording().unwrap_err().to_string();
        assert!(err.contains("Not recording"), "{err}");
        assert!(!state.has_hotkey_release_suppress());
    }

    #[test]
    fn cancel_suppresses_next_hotkey_release() {
        let state = test_state();
        state.force_recording_for_test();
        state.cancel_recording().unwrap();
        assert!(state.has_hotkey_release_suppress());
        // Dictation path: one Released is consumed.
        assert!(state.consume_hotkey_release_suppress());
        assert!(!state.consume_hotkey_release_suppress());
    }

    #[test]
    fn cancel_suppresses_command_release_once() {
        let state = test_state();
        state.force_recording_for_test();
        state.cancel_recording().unwrap();
        // Command path shares the same suppress flag via should_ignore_command_release.
        assert!(state.should_ignore_command_release());
        assert!(!state.should_ignore_command_release());
    }

    #[test]
    fn cancel_clears_command_session_fields() {
        let state = test_state();
        {
            let mut g = state.inner.lock();
            g.status = DictationStatus::Recording;
            g.session_kind = SessionKind::Command;
            g.command_selection = Some("selected text".into());
            g.command_ignore_release_until = Some(Instant::now() + Duration::from_secs(10));
        }
        state.cancel_recording().unwrap();
        let snap = state.snapshot();
        assert_eq!(snap.status, DictationStatus::Idle);
        assert_eq!(snap.session_kind, "dictation");
        let g = state.inner.lock();
        assert!(g.command_selection.is_none());
        assert!(g.command_ignore_release_until.is_none());
    }

    #[test]
    fn new_session_clears_stale_release_suppress() {
        // Cancel without a held chord must not poison the *next* session's release.
        let state = test_state();
        state.force_recording_for_test();
        state.cancel_recording().unwrap();
        assert!(state.has_hotkey_release_suppress());
        state.force_recording_for_test();
        assert!(
            !state.has_hotkey_release_suppress(),
            "starting a new session should clear post-cancel release suppress"
        );
    }

    #[test]
    fn note_recording_mic_sets_label_and_fallback_log() {
        let state = test_state();
        let info = audio::MicOpenInfo::from_resolved(&audio::ResolvedInput::FallbackDefault {
            preferred: "Gone Mic".into(),
        });
        {
            let mut g = state.inner.lock();
            AppState::note_recording_mic(&mut g, &info);
        }
        let snap = state.snapshot();
        assert_eq!(
            snap.last_input_device_label.as_deref(),
            Some(info.device_label.as_str())
        );
        assert_eq!(
            snap.last_mic_fallback_notice.as_deref(),
            Some("Preferred mic \"Gone Mic\" unavailable — using system default")
        );
        assert!(
            snap.log
                .iter()
                .any(|l| l.contains("Preferred mic") && l.contains("unavailable")),
            "expected fallback log line, got {:?}",
            snap.log
        );
    }

    #[test]
    fn note_recording_mic_system_default_no_fallback_notice() {
        let state = test_state();
        {
            let mut g = state.inner.lock();
            AppState::note_recording_mic(&mut g, &audio::MicOpenInfo::system_default());
        }
        let snap = state.snapshot();
        assert_eq!(
            snap.last_input_device_label.as_deref(),
            Some("system default")
        );
        assert!(snap.last_mic_fallback_notice.is_none());
        assert!(
            !snap
                .log
                .iter()
                .any(|l| l.contains("Preferred mic") && l.contains("unavailable")),
            "system default must not emit fallback notice: {:?}",
            snap.log
        );
    }

    #[test]
    fn note_recording_mic_named_with_unavailable_in_name_is_not_fallback() {
        // Device name substring must not trigger fallback (structured flag only).
        let state = test_state();
        let info = audio::MicOpenInfo::from_resolved(&audio::ResolvedInput::Named(
            "Mic unavailable for studio".into(),
        ));
        {
            let mut g = state.inner.lock();
            AppState::note_recording_mic(&mut g, &info);
        }
        let snap = state.snapshot();
        assert_eq!(
            snap.last_input_device_label.as_deref(),
            Some("Mic unavailable for studio")
        );
        assert!(snap.last_mic_fallback_notice.is_none());
    }

    #[test]
    fn snapshot_input_device_defaults() {
        let state = test_state();
        let snap = state.snapshot();
        assert!(snap.input_device_name.is_none());
        assert!(snap.last_input_device_label.is_none());
        assert!(snap.last_mic_fallback_notice.is_none());
    }

    #[test]
    fn snapshot_silence_trim_defaults_on() {
        let state = test_state();
        // Isolate from whatever is on disk under the real settings path.
        {
            let mut g = state.inner.lock();
            g.settings.silence_trim = true;
        }
        let snap = state.snapshot();
        assert!(snap.silence_trim);
    }

    #[test]
    fn set_silence_trim_updates_snapshot_and_log() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("eaglescribe-trim-test-{nanos}.json"));

        let state = test_state();
        {
            let mut g = state.inner.lock();
            g.settings_path = path.clone();
            g.settings = AppSettings::default();
        }

        state.set_silence_trim(false).expect("disable");
        assert!(!state.snapshot().silence_trim);
        let snap = state.snapshot();
        assert!(
            snap.log
                .iter()
                .any(|l| l.contains("Silence trim") && l.contains("off")),
            "expected silence trim off log, got {:?}",
            snap.log
        );

        let loaded = AppSettings::load(&path).expect("load saved settings");
        assert!(!loaded.silence_trim);

        state.set_silence_trim(true).expect("enable");
        assert!(state.snapshot().silence_trim);
        let loaded_on = AppSettings::load(&path).expect("load after on");
        assert!(loaded_on.silence_trim);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn snapshot_menu_bar_only_defaults_off() {
        let state = test_state();
        // Isolate from whatever is on disk under the real settings path.
        {
            let mut g = state.inner.lock();
            g.settings.menu_bar_only = false;
        }
        let snap = state.snapshot();
        assert!(!snap.menu_bar_only);
        assert_eq!(snap.menu_bar_only_available, cfg!(target_os = "macos"));
        assert!(!state.menu_bar_only());
    }

    #[test]
    fn snapshot_reports_compile_time_stt_accel() {
        let snap = test_state().snapshot();
        assert!(
            matches!(snap.stt_accel.as_str(), "metal" | "cuda" | "vulkan" | "cpu"),
            "unexpected stt_accel {:?}",
            snap.stt_accel
        );
        assert_eq!(snap.stt_accel, stt::stt_acceleration());
        assert_eq!(snap.show_metal_rebuild_hint, stt::show_metal_rebuild_hint());
        // No runtime probe — CPU default builds always report "cpu".
        if !cfg!(any(feature = "metal", feature = "cuda", feature = "vulkan")) {
            assert_eq!(snap.stt_accel, "cpu");
        }
    }

    #[test]
    fn snapshot_global_hotkeys_ok_defaults_false_until_registration() {
        let state = test_state();
        let snap = state.snapshot();
        assert!(
            !snap.global_hotkeys_ok,
            "must not claim hotkeys active before OS registration"
        );
        assert!(
            matches!(
                snap.linux_session.as_str(),
                "x11" | "wayland" | "other" | "unknown"
            ),
            "unexpected linux_session {:?}",
            snap.linux_session
        );
        if !cfg!(target_os = "linux") {
            assert_eq!(snap.linux_session, "unknown");
        }

        state.set_global_hotkeys_ok(true);
        assert!(state.snapshot().global_hotkeys_ok);
        assert!(state.snapshot().dictation_hotkey_ok);
        assert!(state.snapshot().command_hotkey_ok);

        state.set_global_hotkeys_ok(false);
        assert!(!state.snapshot().global_hotkeys_ok);

        // Partial: dictation live, command not — must not claim full ok.
        state.set_hotkey_registration(true, false);
        let snap = state.snapshot();
        assert!(snap.dictation_hotkey_ok);
        assert!(!snap.command_hotkey_ok);
        assert!(!snap.global_hotkeys_ok);
    }

    #[test]
    fn set_menu_bar_only_updates_snapshot_and_log() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("eaglescribe-mbo-test-{nanos}.json"));

        let state = test_state();
        // Point saves at a temp file so we do not touch the user's settings.json.
        {
            let mut g = state.inner.lock();
            g.settings_path = path.clone();
            g.settings = AppSettings::default();
        }

        state.set_menu_bar_only(true).expect("enable");
        assert!(state.menu_bar_only());
        let snap = state.snapshot();
        assert!(snap.menu_bar_only);
        assert!(
            snap.log.iter().any(|l| l.contains("Menu bar only")
                && l.contains("on")
                && l.contains("next launch")),
            "expected restart-required log, got {:?}",
            snap.log
        );

        // Round-trip through the same path the setter just wrote.
        let loaded = AppSettings::load(&path).expect("load saved settings");
        assert!(loaded.menu_bar_only);

        state.set_menu_bar_only(false).expect("disable");
        assert!(!state.menu_bar_only());
        assert!(!state.snapshot().menu_bar_only);
        let loaded_off = AppSettings::load(&path).expect("load after off");
        assert!(!loaded_off.menu_bar_only);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn snapshot_onboarding_dismissed_defaults_false() {
        let state = test_state();
        {
            let mut g = state.inner.lock();
            g.settings.onboarding_dismissed = false;
        }
        let snap = state.snapshot();
        assert!(!snap.onboarding_dismissed);
        assert_eq!(snap.host_os, host_os_label());
        assert!(
            matches!(snap.host_os.as_str(), "macos" | "linux" | "other"),
            "unexpected host_os {:?}",
            snap.host_os
        );
    }

    #[test]
    fn set_onboarding_dismissed_updates_snapshot_and_log() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("eaglescribe-onboard-test-{nanos}.json"));

        let state = test_state();
        {
            let mut g = state.inner.lock();
            g.settings_path = path.clone();
            g.settings = AppSettings::default();
        }

        state.set_onboarding_dismissed(true).expect("dismiss");
        assert!(state.snapshot().onboarding_dismissed);
        let snap = state.snapshot();
        assert!(
            snap.log
                .iter()
                .any(|l| l.contains("Setup checklist") && l.contains("dismissed")),
            "expected dismiss log, got {:?}",
            snap.log
        );

        let loaded = AppSettings::load(&path).expect("load saved settings");
        assert!(loaded.onboarding_dismissed);

        state.set_onboarding_dismissed(false).expect("reset");
        assert!(!state.snapshot().onboarding_dismissed);
        let loaded_off = AppSettings::load(&path).expect("load after reset");
        assert!(!loaded_off.onboarding_dismissed);

        let _ = std::fs::remove_file(&path);
    }

    /// Failure-time help must still classify when onboarding was dismissed (AC 8–9).
    #[test]
    fn snapshot_permissions_help_from_last_error_ignores_dismiss() {
        let state = test_state();
        {
            let mut g = state.inner.lock();
            g.settings.onboarding_dismissed = true;
            g.last_error = None;
        }
        assert!(state.snapshot().permissions_help.is_none());

        {
            let mut g = state.inner.lock();
            g.last_error = Some(
                "No audio captured — check microphone permissions".into(),
            );
        }
        let snap = state.snapshot();
        assert!(snap.onboarding_dismissed);
        assert_eq!(snap.permissions_help.as_deref(), Some("microphone"));

        {
            let mut g = state.inner.lock();
            g.last_error = Some(
                "Paste failed — transcript left on clipboard. Paste manually with Cmd/Ctrl+V."
                    .into(),
            );
        }
        assert_eq!(
            state.snapshot().permissions_help.as_deref(),
            Some("accessibility")
        );

        {
            let mut g = state.inner.lock();
            g.last_error = Some("Empty transcript (try speaking longer)".into());
        }
        assert!(state.snapshot().permissions_help.is_none());
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusSnapshot {
    pub status: DictationStatus,
    pub model_path: String,
    pub model_loaded: bool,
    pub polish_mode: PolishMode,
    pub hotkey_mode: HotkeyMode,
    pub dictation_hotkey: String,
    pub command_hotkey: String,
    pub llm_base_url: String,
    pub llm_model: String,
    pub dictionary_path: String,
    pub dictionary: Vec<DictEntry>,
    pub snippets_path: String,
    pub snippets: Vec<Snippet>,
    pub history_path: String,
    pub history_enabled: bool,
    pub history_max: usize,
    pub history: Vec<HistoryEntry>,
    /// When true, previous clipboard text is restored after a successful paste.
    pub clipboard_restore: bool,
    /// When true, leading/trailing silence is trimmed before Whisper.
    pub silence_trim: bool,
    /// Persist preference: hide Dock icon (macOS Accessory) on next launch.
    pub menu_bar_only: bool,
    /// True when this build can apply menu-bar-only (macOS only).
    pub menu_bar_only_available: bool,
    /// Preferred microphone name; `None` means system default.
    pub input_device_name: Option<String>,
    /// Open-time mic label from the last recording start.
    pub last_input_device_label: Option<String>,
    /// Backend-computed notice when preferred mic fell back (UI displays; do not re-parse labels).
    pub last_mic_fallback_notice: Option<String>,
    pub last_transcript: Option<String>,
    pub last_raw_transcript: Option<String>,
    pub last_error: Option<String>,
    /// Failure-time permissions help code: `microphone` | `accessibility` | `model`.
    /// Derived from `last_error`; independent of `onboarding_dismissed`.
    pub permissions_help: Option<String>,
    pub log: Vec<String>,
    pub session_kind: String,
    /// Compile-time STT acceleration: `metal` | `cuda` | `vulkan` | `cpu`.
    pub stt_accel: String,
    /// Soft UI hint: Apple Silicon + CPU-only build (rebuild with Metal).
    pub show_metal_rebuild_hint: bool,
    /// True when both dictation and command global hotkeys registered with the OS.
    /// False when registration failed or was partial — UI must not claim *all* shortcuts are active.
    pub global_hotkeys_ok: bool,
    /// Dictation global shortcut registered with the OS.
    pub dictation_hotkey_ok: bool,
    /// Command Mode global shortcut registered with the OS.
    pub command_hotkey_ok: bool,
    /// Linux session type probe: `x11` | `wayland` | `other` | `unknown` (always set).
    pub linux_session: String,
    /// When true, first-run setup checklist should not auto-show.
    pub onboarding_dismissed: bool,
    /// Compile-time host: `macos` | `linux` | `other` (permissions copy branches).
    pub host_os: String,
}

/// Host OS token for UI onboarding / permissions copy.
fn host_os_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "other"
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

pub type SharedState = Arc<AppState>;
