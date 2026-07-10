use crate::audio::RecordingSession;
use crate::dictionary::{self, DictEntry, Dictionary};
use crate::error::{AppError, AppResult};
use crate::polish::{self, PolishMode};
use crate::settings::{self, AppSettings, HotkeyMode};
use crate::snippets::{self, Snippet, SnippetBook};
use crate::stt::WhisperEngine;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationStatus {
    Idle,
    Recording,
    Transcribing,
    Error,
}

pub struct AppState {
    inner: Mutex<InnerState>,
}

struct InnerState {
    status: DictationStatus,
    model_path: PathBuf,
    settings_path: PathBuf,
    settings: AppSettings,
    dictionary_path: PathBuf,
    dictionary: Dictionary,
    snippets_path: PathBuf,
    snippets: SnippetBook,
    engine: Option<WhisperEngine>,
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

        let hotkey_mode = settings.hotkey_mode;

        Self {
            inner: Mutex::new(InnerState {
                status: DictationStatus::Idle,
                model_path,
                settings_path: settings_path.clone(),
                settings,
                dictionary_path: dictionary_path.clone(),
                dictionary,
                snippets_path: snippets_path.clone(),
                snippets,
                engine: None,
                session: None,
                polish_mode: PolishMode::Smart,
                last_transcript: None,
                last_raw_transcript: None,
                last_error: None,
                log: {
                    let mut log = vec!["TalonType ready.".into()];
                    log.push(format!(
                        "Hotkey mode: {} ({})",
                        hotkey_mode.as_str(),
                        hotkey_mode.label()
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
                    log
                },
            }),
        }
    }

    pub fn snapshot(&self) -> StatusSnapshot {
        let g = self.inner.lock();
        StatusSnapshot {
            status: g.status,
            model_path: g.model_path.display().to_string(),
            model_loaded: g.engine.is_some(),
            polish_mode: g.polish_mode,
            hotkey_mode: g.settings.hotkey_mode,
            dictionary_path: g.dictionary_path.display().to_string(),
            dictionary: g.dictionary.list(),
            snippets_path: g.snippets_path.display().to_string(),
            snippets: g.snippets.list(),
            last_transcript: g.last_transcript.clone(),
            last_raw_transcript: g.last_raw_transcript.clone(),
            last_error: g.last_error.clone(),
            log: g.log.clone(),
        }
    }

    pub fn hotkey_mode(&self) -> HotkeyMode {
        self.inner.lock().settings.hotkey_mode
    }

    pub fn push_log(&self, msg: impl Into<String>) {
        let mut g = self.inner.lock();
        let msg = msg.into();
        eprintln!("[talontype] {msg}");
        g.log.push(msg);
        if g.log.len() > 100 {
            let drain = g.log.len() - 100;
            g.log.drain(0..drain);
        }
    }

    pub fn set_model_path(&self, path: PathBuf) {
        let mut g = self.inner.lock();
        g.model_path = path;
        g.engine = None;
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
        g.engine = Some(engine);
        g.log.push("Whisper model loaded.".into());
        Ok(())
    }

    pub fn start_recording(&self) -> AppResult<()> {
        let mut g = self.inner.lock();
        if g.status == DictationStatus::Recording {
            return Err(AppError::from("Already recording"));
        }
        if g.status == DictationStatus::Transcribing {
            return Err(AppError::from("Busy transcribing"));
        }

        let session = RecordingSession::start()?;
        g.session = Some(session);
        g.status = DictationStatus::Recording;
        g.last_error = None;
        g.log
            .push("Recording… (release hotkey or use Stop to finish)".into());
        Ok(())
    }

    /// Stop mic, transcribe, polish, dictionary, snippets, inject.
    pub fn stop_and_transcribe<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<String> {
        let session = {
            let mut g = self.inner.lock();
            if g.status != DictationStatus::Recording {
                return Err(AppError::from("Not recording"));
            }
            g.session
                .take()
                .ok_or_else(|| AppError::from("Missing recording session"))?
        };

        if let Err(e) = self.ensure_engine() {
            let mut g = self.inner.lock();
            g.status = DictationStatus::Error;
            g.last_error = Some(e.to_string());
            return Err(e);
        }

        {
            let mut g = self.inner.lock();
            g.status = DictationStatus::Transcribing;
            g.log.push("Transcribing on-device…".into());
        }

        let (samples, rate) = match session.stop() {
            Ok(v) => v,
            Err(e) => {
                let mut g = self.inner.lock();
                g.status = DictationStatus::Error;
                g.last_error = Some(e.to_string());
                return Err(e);
            }
        };

        let audio = crate::audio::resample_to_16k(&samples, rate);
        let duration_s = audio.len() as f32 / 16_000.0;
        self.push_log(format!(
            "Captured {duration_s:.1}s audio ({} samples @ 16 kHz)",
            audio.len()
        ));

        let raw = {
            let g = self.inner.lock();
            let engine = g
                .engine
                .as_ref()
                .ok_or_else(|| AppError::from("Engine not loaded"))?;
            engine.transcribe_16k_mono(&audio)
        };

        let raw = match raw {
            Ok(t) => t,
            Err(e) => {
                let mut g = self.inner.lock();
                g.status = DictationStatus::Error;
                g.last_error = Some(e.to_string());
                return Err(e);
            }
        };

        if raw.trim().is_empty() {
            let mut g = self.inner.lock();
            g.status = DictationStatus::Idle;
            g.last_error = Some("Empty transcript (try speaking longer)".into());
            g.log.push("Empty transcript.".into());
            return Err(AppError::from("Empty transcript"));
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
            let mut g = self.inner.lock();
            g.status = DictationStatus::Idle;
            g.last_raw_transcript = Some(polished.raw);
            g.last_error = Some("Transcript empty after polish".into());
            return Err(AppError::from("Transcript empty after polish"));
        }

        match crate::inject::inject_text(app, &text) {
            Ok(result) => {
                let mut g = self.inner.lock();
                g.last_raw_transcript = Some(polished.raw);
                g.last_transcript = Some(result.text.clone());
                g.status = DictationStatus::Idle;
                if result.pasted {
                    g.log
                        .push(format!("Injected: {}", truncate(&result.text, 80)));
                } else {
                    g.log.push(format!(
                        "Copied (paste manually with Cmd/Ctrl+V): {}",
                        truncate(&result.text, 80)
                    ));
                }
                Ok(result.text)
            }
            Err(e) => {
                let _ = crate::inject::copy_to_clipboard(&text);
                let mut g = self.inner.lock();
                g.last_raw_transcript = Some(polished.raw);
                g.last_transcript = Some(text.clone());
                g.status = DictationStatus::Idle;
                g.last_error = Some(e.to_string());
                g.log
                    .push(format!("Transcript on clipboard; inject failed: {e}"));
                Ok(text)
            }
        }
    }

    pub fn cancel_recording(&self) -> AppResult<()> {
        let mut g = self.inner.lock();
        if g.status != DictationStatus::Recording {
            return Err(AppError::from("Not recording"));
        }
        let _ = g.session.take();
        g.status = DictationStatus::Idle;
        g.log.push("Recording cancelled.".into());
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusSnapshot {
    pub status: DictationStatus,
    pub model_path: String,
    pub model_loaded: bool,
    pub polish_mode: PolishMode,
    pub hotkey_mode: HotkeyMode,
    pub dictionary_path: String,
    pub dictionary: Vec<DictEntry>,
    pub snippets_path: String,
    pub snippets: Vec<Snippet>,
    pub last_transcript: Option<String>,
    pub last_raw_transcript: Option<String>,
    pub last_error: Option<String>,
    pub log: Vec<String>,
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
