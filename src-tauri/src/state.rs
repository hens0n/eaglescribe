use crate::audio::RecordingSession;
use crate::dictionary::{self, DictEntry, Dictionary};
use crate::error::{AppError, AppResult};
use crate::history::{self, HistoryBook, HistoryEntry};
use crate::hotkey;
use crate::llm;
use crate::polish::{self, PolishMode};
use crate::settings::{self, AppSettings, HotkeyMode};
use crate::snippets::{self, Snippet, SnippetBook};
use crate::stt::WhisperEngine;
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
                log: {
                    let mut log = vec!["EagleScribe ready.".into()];
                    log.push(format!(
                        "Hotkey mode: {} ({})",
                        hotkey_mode.as_str(),
                        hotkey_mode.label()
                    ));
                    log.push(format!("Dictation hotkey: {dictation_hotkey}"));
                    log.push(format!(
                        "Command Mode: {command_hotkey} (select text, hold, speak instruction)"
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
            last_transcript: g.last_transcript.clone(),
            last_raw_transcript: g.last_raw_transcript.clone(),
            last_error: g.last_error.clone(),
            log: g.log.clone(),
            session_kind: match g.session_kind {
                SessionKind::Dictation => "dictation".into(),
                SessionKind::Command => "command".into(),
            },
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
        g.log.push(format!(
            "Hotkey mode: {} ({})",
            mode.as_str(),
            mode.label()
        ));
        Ok(())
    }

    /// Persist validated hotkey combos (caller re-registers OS shortcuts).
    pub fn set_hotkey_bindings(&self, dictation: &str, command: &str) -> AppResult<()> {
        let (dictation, command) = hotkey::validate_pair(dictation, command)?;
        let mut g = self.inner.lock();
        g.settings.dictation_hotkey = dictation.clone();
        g.settings.command_hotkey = command.clone();
        g.settings.save(&g.settings_path)?;
        g.log
            .push(format!("Dictation hotkey: {dictation} · Command: {command}"));
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
        g.log.push(format!(
            "Dictionary: {:?} → {:?}",
            from.trim(),
            to.trim()
        ));
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
        g.log
            .push(format!("Dictionary removed: {:?}", from.trim()));
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
        g.log
            .push(format!("Snippet removed: {:?}", cue.trim()));
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
        let mut g = self.inner.lock();
        Self::reject_if_unavailable(g.status)?;

        let session = RecordingSession::start()?;
        g.session = Some(session);
        g.session_kind = SessionKind::Dictation;
        g.command_selection = None;
        g.status = DictationStatus::Recording;
        g.last_error = None;
        g.log
            .push("Recording… (release hotkey or use Stop to finish)".into());
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

            let session = RecordingSession::start()?;
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
            g.log.push(
                "Command Mode recording… speak your instruction (e.g. make this more concise)"
                    .into(),
            );
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

        let (samples, rate) = match session.stop() {
            Ok(v) => v,
            Err(e) => {
                self.fail_status(app, DictationStatus::Error, e.to_string());
                return Err(e);
            }
        };

        let audio = crate::audio::resample_to_16k(&samples, rate);
        let duration_s = audio.len() as f32 / 16_000.0;
        self.push_log(format!(
            "Captured {duration_s:.1}s audio ({} samples @ 16 kHz)",
            audio.len()
        ));

        // Clone Arc so Whisper runs without holding the state mutex.
        // Holding the mutex during STT deadlocks with main-thread paste/hotkeys.
        let engine = {
            let g = self.inner.lock();
            g.engine
                .clone()
                .ok_or_else(|| AppError::from("Engine not loaded"))?
        };

        let raw = match engine.transcribe_16k_mono(&audio) {
            Ok(t) => t,
            Err(e) => {
                self.fail_status(app, DictationStatus::Error, e.to_string());
                return Err(e);
            }
        };

        if raw.trim().is_empty() {
            {
                let mut g = self.inner.lock();
                g.status = DictationStatus::Idle;
                g.last_error = Some("Empty transcript (try speaking longer)".into());
                g.log.push("Empty transcript.".into());
            }
            let _ = app.emit("dictation-status", self.snapshot());
            return Err(AppError::from("Empty transcript"));
        }

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
        // Paste runs on the main thread; never hold `inner` across this call.
        match crate::inject::inject_text(app, text) {
            Ok(result) => {
                {
                    let mut g = self.inner.lock();
                    g.last_raw_transcript = Some(raw.to_string());
                    g.last_transcript = Some(result.text.clone());
                    g.status = DictationStatus::Idle;
                    g.last_error = None;
                    if result.pasted {
                        g.log
                            .push(format!("Injected: {}", truncate(&result.text, 80)));
                    } else {
                        g.log.push(format!(
                            "Copied (paste manually with Cmd/Ctrl+V): {}",
                            truncate(&result.text, 80)
                        ));
                    }
                    Self::maybe_record_history(
                        &mut g,
                        kind,
                        &result.text,
                        Some(raw),
                    );
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
                    g.last_error = Some(e.to_string());
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

    fn maybe_record_history(
        g: &mut InnerState,
        kind: SessionKind,
        text: &str,
        raw: Option<&str>,
    ) {
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
    pub last_transcript: Option<String>,
    pub last_raw_transcript: Option<String>,
    pub last_error: Option<String>,
    pub log: Vec<String>,
    pub session_kind: String,
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
