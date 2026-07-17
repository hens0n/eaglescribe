//! Shared local raw-recognition pipeline for dictation and Tuning.

use crate::audio;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write;

const RECOGNITION_FINGERPRINT_VERSION: &str = "recognition-v1";
/// Bump whenever resampling, silence detection/trimming, or decoder padding changes.
const PREPROCESSING_BEHAVIOR_VERSION: &str = "linear-16k-trim-v1-tail-400ms";

/// Stable identity of model content plus decoder and audio-preprocessing behavior.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecognitionFingerprint(String);

impl RecognitionFingerprint {
    pub fn from_stable_id(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Compute the identity that gates unmodified Tuning-origin Correction Rules.
pub fn recognition_fingerprint(
    model_content_sha256: &str,
    options: RecognitionOptions,
) -> RecognitionFingerprint {
    let descriptor = format!(
        "model_sha256={model_content_sha256};decoder={};preprocessing={PREPROCESSING_BEHAVIOR_VERSION};silence_trim={};backend={}",
        crate::stt::DECODER_BEHAVIOR_VERSION,
        options.silence_trim,
        crate::stt::stt_acceleration(),
    );
    let digest = Sha256::digest(descriptor.as_bytes());
    let mut encoded = String::with_capacity(RECOGNITION_FINGERPRINT_VERSION.len() + 1 + 64);
    encoded.push_str(RECOGNITION_FINGERPRINT_VERSION);
    encoded.push(':');
    for byte in digest {
        let _ = write!(encoded, "{byte:02x}");
    }
    RecognitionFingerprint(encoded)
}

/// Stage that owns a raw-recognition failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognitionStage {
    Capture,
    Preprocessing,
    Transcription,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognitionErrorKind {
    Capture,
    InvalidAudio,
    SilentAudio,
    Transcription,
    EmptyTranscript,
}

impl RecognitionErrorKind {
    pub fn stage(self) -> RecognitionStage {
        match self {
            Self::Capture => RecognitionStage::Capture,
            Self::InvalidAudio | Self::SilentAudio => RecognitionStage::Preprocessing,
            Self::Transcription | Self::EmptyTranscript => RecognitionStage::Transcription,
        }
    }
}

/// Error with stable stage identity and user-facing detail from the failing operation.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct RecognitionError {
    kind: RecognitionErrorKind,
    message: String,
    preprocessing: Option<PreprocessingReport>,
}

impl RecognitionError {
    fn new(kind: RecognitionErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            preprocessing: None,
        }
    }

    fn after_preprocessing(
        kind: RecognitionErrorKind,
        message: impl Into<String>,
        preprocessing: PreprocessingReport,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            preprocessing: Some(preprocessing),
        }
    }

    pub fn stage(&self) -> RecognitionStage {
        self.kind.stage()
    }

    pub fn kind(&self) -> RecognitionErrorKind {
        self.kind
    }

    /// Completed preprocessing facts, when failure happened during transcription.
    pub fn preprocessing(&self) -> Option<&PreprocessingReport> {
        self.preprocessing.as_ref()
    }
}

pub type RecognitionResult<T> = Result<T, RecognitionError>;

/// Captured mono PCM and the device sample rate that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// Untrimmed mono PCM resampled once to Whisper's required 16 kHz input rate.
pub(crate) struct ResampledAudio16K {
    samples: Vec<f32>,
    input_sample_rate: u32,
}

impl ResampledAudio16K {
    pub(crate) fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub(crate) fn input_sample_rate(&self) -> u32 {
        self.input_sample_rate
    }
}

impl CapturedAudio {
    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
        }
    }

    /// Convert the microphone session result without losing capture-stage identity.
    pub fn from_capture<E>(capture: Result<(Vec<f32>, u32), E>) -> RecognitionResult<Self>
    where
        E: std::fmt::Display,
    {
        capture
            .map(|(samples, sample_rate)| Self::new(samples, sample_rate))
            .map_err(|error| {
                RecognitionError::new(RecognitionErrorKind::Capture, error.to_string())
            })
    }
}

/// Production preprocessing switches that affect raw recognition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecognitionOptions {
    pub silence_trim: bool,
}

impl Default for RecognitionOptions {
    fn default() -> Self {
        Self { silence_trim: true }
    }
}

/// Local transcription boundary used by both Whisper and deterministic tests.
pub trait RawTranscriber {
    fn transcribe_16k_mono(&self, audio: &[f32]) -> Result<String, String>;
}

impl RawTranscriber for crate::stt::WhisperEngine {
    fn transcribe_16k_mono(&self, audio: &[f32]) -> Result<String, String> {
        crate::stt::WhisperEngine::transcribe_16k_mono(self, audio)
            .map_err(|error| error.to_string())
    }
}

/// Raw transcript plus preprocessing facts useful for content-free diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct RawRecognition {
    pub transcript: String,
    pub preprocessing: PreprocessingReport,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreprocessingReport {
    pub input_sample_rate: u32,
    pub captured_samples_16k: usize,
    pub peak: f32,
    pub silence_trim: SilenceTrimReport,
    pub decoder_tail_padding_ms: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SilenceTrimReport {
    Disabled,
    Applied {
        original_ms: u32,
        trimmed_ms: u32,
        head_ms: u32,
        tail_ms: u32,
        threshold: f32,
    },
    KeptFullBuffer {
        original_ms: u32,
        remaining_ms: u32,
        threshold: f32,
    },
}

/// Run captured audio through production resampling, optional silence trimming,
/// decoder padding, and local transcription. No dictation side effects run here.
pub fn recognize_raw(
    capture: CapturedAudio,
    options: RecognitionOptions,
    transcriber: &impl RawTranscriber,
) -> RecognitionResult<RawRecognition> {
    let audio = resample_capture(capture)?;
    recognize_resampled(audio, options, transcriber)
}

pub(crate) fn resample_capture(capture: CapturedAudio) -> RecognitionResult<ResampledAudio16K> {
    if capture.sample_rate == 0 {
        return Err(RecognitionError::new(
            RecognitionErrorKind::InvalidAudio,
            "Microphone never reported a sample rate",
        ));
    }
    let input_sample_rate = capture.sample_rate;
    let samples = audio::resample_to_16k(&capture.samples, input_sample_rate);
    Ok(ResampledAudio16K {
        samples,
        input_sample_rate,
    })
}

pub(crate) fn recognize_resampled(
    audio_16k: ResampledAudio16K,
    options: RecognitionOptions,
    transcriber: &impl RawTranscriber,
) -> RecognitionResult<RawRecognition> {
    let input_sample_rate = audio_16k.input_sample_rate;
    let mut samples = audio_16k.samples;
    let captured_samples_16k = samples.len();
    let peak = audio::peak_abs(&samples);
    if samples.is_empty() || peak < audio::SILENT_CAPTURE_PEAK {
        return Err(RecognitionError::new(
            RecognitionErrorKind::SilentAudio,
            format!(
                "No audio captured — check microphone permissions (peak={peak:.4}). System Settings → Privacy & Security → Microphone → enable EagleScribe."
            ),
        ));
    }
    let silence_trim = if options.silence_trim {
        let threshold = audio::speech_rms_threshold(&samples);
        match audio::trim_silence_16k(&samples) {
            audio::TrimOutcome::Ok(trimmed) => {
                let report = SilenceTrimReport::Applied {
                    original_ms: trimmed.original_ms,
                    trimmed_ms: trimmed.trimmed_ms,
                    head_ms: trimmed.head_ms,
                    tail_ms: trimmed.tail_ms,
                    threshold,
                };
                samples = trimmed.samples;
                report
            }
            audio::TrimOutcome::BelowFloor {
                original_ms,
                remaining_ms,
            } => SilenceTrimReport::KeptFullBuffer {
                original_ms,
                remaining_ms,
                threshold,
            },
        }
    } else {
        SilenceTrimReport::Disabled
    };
    let samples = audio::pad_for_whisper_16k(&samples);
    let preprocessing = PreprocessingReport {
        input_sample_rate,
        captured_samples_16k,
        peak,
        silence_trim,
        decoder_tail_padding_ms: audio::WHISPER_TAIL_PAD_MS,
    };
    let transcript = transcriber.transcribe_16k_mono(&samples).map_err(|error| {
        RecognitionError::after_preprocessing(
            RecognitionErrorKind::Transcription,
            error.to_string(),
            preprocessing.clone(),
        )
    })?;
    if transcript.trim().is_empty() {
        return Err(RecognitionError::after_preprocessing(
            RecognitionErrorKind::EmptyTranscript,
            "Empty transcript",
            preprocessing,
        ));
    }
    Ok(RawRecognition {
        transcript,
        preprocessing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct RecordingTranscriber {
        received: RefCell<Vec<f32>>,
        result: Result<String, String>,
    }

    #[test]
    fn recognition_fingerprint_tracks_model_and_preprocessing_behavior() {
        let trimmed = RecognitionOptions { silence_trim: true };
        let untrimmed = RecognitionOptions {
            silence_trim: false,
        };

        let first = recognition_fingerprint("model-content-a", trimmed);

        assert_eq!(first, recognition_fingerprint("model-content-a", trimmed));
        assert_ne!(first, recognition_fingerprint("model-content-b", trimmed));
        assert_ne!(first, recognition_fingerprint("model-content-a", untrimmed));
        assert!(first.as_str().starts_with("recognition-v1:"));
    }

    impl RawTranscriber for RecordingTranscriber {
        fn transcribe_16k_mono(&self, audio: &[f32]) -> Result<String, String> {
            self.received.replace(audio.to_vec());
            self.result.clone()
        }
    }

    #[test]
    fn caller_gets_raw_transcript_through_production_preprocessing() {
        let transcriber = RecordingTranscriber {
            received: RefCell::new(Vec::new()),
            result: Ok("um meet at two actually three".into()),
        };
        let capture = CapturedAudio::new(vec![0.01; 8_000], 8_000);

        let result = recognize_raw(capture, RecognitionOptions::default(), &transcriber)
            .expect("raw recognition should succeed");

        assert_eq!(result.transcript, "um meet at two actually three");
        assert_eq!(result.preprocessing.captured_samples_16k, 16_000);
        assert_eq!(result.preprocessing.decoder_tail_padding_ms, 400);
        let received = transcriber.received.borrow();
        assert_eq!(received.len(), 16_000 + 6_400);
        assert!(received[16_000..].iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn capture_failure_is_reported_at_the_capture_stage() {
        let error = CapturedAudio::from_capture(Err("microphone disconnected"))
            .expect_err("capture should fail");

        assert_eq!(error.stage(), RecognitionStage::Capture);
        assert_eq!(error.to_string(), "microphone disconnected");
    }

    #[test]
    fn silent_capture_is_reported_at_the_preprocessing_stage() {
        let transcriber = RecordingTranscriber {
            received: RefCell::new(Vec::new()),
            result: Ok("should not be called".into()),
        };

        let error = recognize_raw(
            CapturedAudio::new(vec![0.0; 16_000], 16_000),
            RecognitionOptions::default(),
            &transcriber,
        )
        .expect_err("silent captured audio should fail before transcription");

        assert_eq!(error.stage(), RecognitionStage::Preprocessing);
        assert!(error.to_string().contains("No audio captured"));
        assert!(transcriber.received.borrow().is_empty());
    }

    #[test]
    fn transcriber_failure_is_reported_at_the_transcription_stage() {
        let transcriber = RecordingTranscriber {
            received: RefCell::new(Vec::new()),
            result: Err("Whisper inference failed: decoder stopped".into()),
        };

        let error = recognize_raw(
            CapturedAudio::new(vec![0.01; 16_000], 16_000),
            RecognitionOptions::default(),
            &transcriber,
        )
        .expect_err("transcription should fail");

        assert_eq!(error.stage(), RecognitionStage::Transcription);
        assert_eq!(error.kind(), RecognitionErrorKind::Transcription);
        assert!(error.to_string().contains("decoder stopped"));
        let preprocessing = error
            .preprocessing()
            .expect("completed preprocessing should remain available on transcription failure");
        assert_eq!(preprocessing.captured_samples_16k, 16_000);
        assert_eq!(preprocessing.decoder_tail_padding_ms, 400);
    }

    #[test]
    fn disabling_silence_trim_keeps_the_full_resampled_capture() {
        let transcriber = RecordingTranscriber {
            received: RefCell::new(Vec::new()),
            result: Ok("raw words".into()),
        };
        let mut samples = vec![0.0; 4_000];
        samples.extend(vec![0.01; 8_000]);
        samples.extend(vec![0.0; 4_000]);

        let result = recognize_raw(
            CapturedAudio::new(samples, 16_000),
            RecognitionOptions {
                silence_trim: false,
            },
            &transcriber,
        )
        .expect("raw recognition should succeed");

        assert_eq!(
            result.preprocessing.silence_trim,
            SilenceTrimReport::Disabled
        );
        assert_eq!(transcriber.received.borrow().len(), 16_000 + 6_400);
    }
}
