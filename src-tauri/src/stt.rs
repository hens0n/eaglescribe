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
    ///
    /// Callers should pass audio that already includes a short trailing silence
    /// pad (see [`crate::audio::pad_for_whisper_16k`]) so the decoder does not
    /// clip the last words of a long or multi-sentence take.
    pub fn transcribe_16k_mono(&self, audio: &[f32]) -> AppResult<String> {
        if audio.is_empty() {
            return Err(AppError::from("Empty audio buffer"));
        }

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| AppError::from(format!("Whisper state error: {e}")))?;

        // Beam search is slower than greedy but far less likely to emit an early
        // EOT mid-sentence on base.en — the main cause of "second sentence cut off".
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: 1.0,
        });
        params.set_n_threads(num_threads());
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_translate(false);
        params.set_suppress_blank(true);
        // Keep non-speech token suppression on so music/noise tags are less likely.
        params.set_suppress_nst(true);
        // Do not condition on prior text across windows — dictation takes are
        // independent and prior context can cause loops on short clips.
        params.set_no_context(true);
        // Temperature fallbacks (defaults: 0 + 0.2 inc) retry low-quality passes.
        params.set_temperature(0.0);
        params.set_temperature_inc(0.2);

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
            let segment = clean_segment_text(&segment);
            if segment.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&segment);
        }

        Ok(text)
    }
}

/// Strip Whisper hallucination tags and normalize whitespace on one segment.
fn clean_segment_text(segment: &str) -> String {
    let mut s = segment.trim().to_string();
    // base.en sometimes emits these instead of (or mixed into) real speech.
    for tag in [
        "[BLANK_AUDIO]",
        "[blank_audio]",
        "[MUSIC]",
        "[music]",
        "[NOISE]",
        "[noise]",
        "[Silence]",
        "[silence]",
        "(blank)",
        "(silence)",
    ] {
        s = s.replace(tag, " ");
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn num_threads() -> std::ffi::c_int {
    std::thread::available_parallelism()
        .map(|n| n.get() as std::ffi::c_int)
        .unwrap_or(4)
        .clamp(1, 8)
}

/// Compile-time STT acceleration backend for the running binary.
///
/// Driven only by Cargo features (`metal` / `cuda` / `vulkan`); never runtime GPU probes.
/// Priority if multiple features were enabled: metal → cuda → vulkan → cpu.
pub fn stt_acceleration() -> &'static str {
    if cfg!(feature = "metal") {
        "metal"
    } else if cfg!(feature = "cuda") {
        "cuda"
    } else if cfg!(feature = "vulkan") {
        "vulkan"
    } else {
        "cpu"
    }
}

/// True on Apple Silicon builds that did not compile Metal into Whisper.
///
/// Used for a soft Settings hint only — never blocks dictation or model load.
pub fn show_metal_rebuild_hint() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64")) && stt_acceleration() == "cpu"
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
        return repo_models.canonicalize().unwrap_or(repo_models);
    }

    if let Some(data) = dirs::data_local_dir() {
        return data
            .join("eaglescribe")
            .join("models")
            .join("ggml-base.en.bin");
    }

    PathBuf::from("models/ggml-base.en.bin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stt_acceleration_is_known_label() {
        let a = stt_acceleration();
        assert!(
            matches!(a, "metal" | "cuda" | "vulkan" | "cpu"),
            "unexpected stt_accel {a:?}"
        );
    }

    #[test]
    fn clean_segment_strips_blank_audio_tags() {
        assert_eq!(clean_segment_text("[BLANK_AUDIO]"), "");
        assert_eq!(
            clean_segment_text("hello [BLANK_AUDIO] world"),
            "hello world"
        );
        assert_eq!(clean_segment_text("  dictation is working.  "), "dictation is working.");
    }

    #[test]
    fn stt_acceleration_matches_compile_features() {
        // Default CI/dev builds are CPU; feature builds report their backend.
        if cfg!(feature = "metal") {
            assert_eq!(stt_acceleration(), "metal");
        } else if cfg!(feature = "cuda") {
            assert_eq!(stt_acceleration(), "cuda");
        } else if cfg!(feature = "vulkan") {
            assert_eq!(stt_acceleration(), "vulkan");
        } else {
            assert_eq!(stt_acceleration(), "cpu");
        }
    }

    #[test]
    fn metal_rebuild_hint_only_on_as_cpu() {
        let expect =
            cfg!(all(target_os = "macos", target_arch = "aarch64")) && stt_acceleration() == "cpu";
        assert_eq!(show_metal_rebuild_hint(), expect);
        // Metal build must never show the "rebuild with Metal" soft hint.
        if cfg!(feature = "metal") {
            assert!(!show_metal_rebuild_hint());
        }
    }
}
