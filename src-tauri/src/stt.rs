//! On-device speech-to-text via whisper.cpp (`whisper-rs`).

use crate::error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct WhisperEngine {
    ctx: WhisperContext,
    model_content_sha256: String,
}

#[derive(Debug, Clone, Copy)]
struct DecoderBehavior {
    beam_size: std::ffi::c_int,
    patience: f32,
    language: &'static str,
    translate: bool,
    suppress_blank: bool,
    suppress_non_speech_tokens: bool,
    no_context: bool,
    temperature: f32,
    temperature_increment: f32,
}

const PRODUCTION_DECODER: DecoderBehavior = DecoderBehavior {
    beam_size: 5,
    patience: 1.0,
    language: "en",
    translate: false,
    suppress_blank: true,
    suppress_non_speech_tokens: true,
    no_context: true,
    temperature: 0.0,
    temperature_increment: 0.2,
};

const CLEAN_SEGMENT_TAGS: [&str; 10] = [
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
];

/// Exact production decoder/cleanup behavior used by Recognition Fingerprints.
pub fn decoder_behavior_descriptor() -> String {
    let behavior = PRODUCTION_DECODER;
    format!(
        "beam_size={};patience={};language={};translate={};suppress_blank={};suppress_nst={};no_context={};temperature={};temperature_inc={};threads={};clean_tags={}",
        behavior.beam_size,
        behavior.patience,
        behavior.language,
        behavior.translate,
        behavior.suppress_blank,
        behavior.suppress_non_speech_tokens,
        behavior.no_context,
        behavior.temperature,
        behavior.temperature_increment,
        num_threads(),
        CLEAN_SEGMENT_TAGS.join("|"),
    )
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

        let model_content_sha256 = sha256_file(model_path)?;

        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .ok_or_else(|| AppError::from("Model path is not valid UTF-8"))?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| AppError::from(format!("Failed to load Whisper model: {e}")))?;

        Ok(Self {
            ctx,
            model_content_sha256,
        })
    }

    pub fn model_content_sha256(&self) -> &str {
        &self.model_content_sha256
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

        let params = production_decoder_params();

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

fn sha256_file(path: &Path) -> AppResult<String> {
    let mut file = File::open(path)
        .map_err(|error| AppError::from(format!("Read Whisper model failed: {error}")))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| AppError::from(format!("Read Whisper model failed: {error}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn production_decoder_params() -> FullParams<'static, 'static> {
    let behavior = PRODUCTION_DECODER;
    // Beam search is slower than greedy but far less likely to emit an early
    // EOT mid-sentence on base.en — the main cause of "second sentence cut off".
    let mut params = FullParams::new(SamplingStrategy::BeamSearch {
        beam_size: behavior.beam_size,
        patience: behavior.patience,
    });
    params.set_n_threads(num_threads());
    params.set_language(Some(behavior.language));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_translate(behavior.translate);
    params.set_suppress_blank(behavior.suppress_blank);
    params.set_suppress_nst(behavior.suppress_non_speech_tokens);
    params.set_no_context(behavior.no_context);
    params.set_temperature(behavior.temperature);
    params.set_temperature_inc(behavior.temperature_increment);
    params
}

/// Strip Whisper hallucination tags and normalize whitespace on one segment.
fn clean_segment_text(segment: &str) -> String {
    let mut s = segment.trim().to_string();
    // base.en sometimes emits these instead of (or mixed into) real speech.
    for tag in CLEAN_SEGMENT_TAGS {
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
    fn model_content_hash_uses_file_bytes() {
        let path = std::env::temp_dir().join(format!(
            "eaglescribe-model-hash-{}-{}.bin",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::write(&path, b"abc").expect("write model fixture");
        let digest = sha256_file(&path).expect("hash model fixture");
        let _ = std::fs::remove_file(path);
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

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
        assert_eq!(
            clean_segment_text("  dictation is working.  "),
            "dictation is working."
        );
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
