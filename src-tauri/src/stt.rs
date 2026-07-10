//! On-device speech-to-text via whisper.cpp (`whisper-rs`).

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct WhisperEngine {
    ctx: WhisperContext,
}

impl WhisperEngine {
    pub fn load(model_path: impl AsRef<Path>) -> AppResult<Self> {
        let model_path = model_path.as_ref();
        if !model_path.is_file() {
            return Err(AppError::from(format!(
                "Whisper model not found at {}\n\nDownload a ggml model, e.g.:\n  npm run model:download",
                model_path.display()
            )));
        }

        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .ok_or_else(|| AppError::from("Model path is not valid UTF-8"))?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| AppError::from(format!("Failed to load Whisper model: {e}")))?;

        Ok(Self { ctx })
    }

    /// Transcribe mono f32 PCM at 16 kHz.
    pub fn transcribe_16k_mono(&self, audio: &[f32]) -> AppResult<String> {
        if audio.is_empty() {
            return Err(AppError::from("Empty audio buffer"));
        }

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| AppError::from(format!("Whisper state error: {e}")))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(num_threads());
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_translate(false);
        params.set_suppress_blank(true);
        params.set_no_context(true);

        state
            .full(params, audio)
            .map_err(|e| AppError::from(format!("Whisper inference failed: {e}")))?;

        let n = state
            .full_n_segments()
            .map_err(|e| AppError::from(format!("Segment count failed: {e}")))?;
        let mut text = String::new();
        for i in 0..n {
            let segment = state
                .full_get_segment_text(i)
                .map_err(|e| AppError::from(format!("Segment text failed: {e}")))?;
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(segment);
        }

        Ok(text)
    }
}

fn num_threads() -> std::ffi::c_int {
    std::thread::available_parallelism()
        .map(|n| n.get() as std::ffi::c_int)
        .unwrap_or(4)
        .clamp(1, 8)
}

/// Resolve model path: explicit override → env → default under repo models / app data.
pub fn resolve_model_path(override_path: Option<&str>) -> PathBuf {
    if let Some(p) = override_path {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }

    if let Ok(env_path) = std::env::var("EAGLESCRIBE_WHISPER_MODEL") {
        if !env_path.is_empty() {
            return PathBuf::from(env_path);
        }
    }

    let repo_models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/ggml-base.en.bin");
    if repo_models.is_file() {
        return repo_models
            .canonicalize()
            .unwrap_or(repo_models);
    }

    if let Some(data) = dirs::data_local_dir() {
        return data.join("eaglescribe").join("models").join("ggml-base.en.bin");
    }

    PathBuf::from("models/ggml-base.en.bin")
}
