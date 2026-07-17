//! Microphone capture for dictation spikes.
//!
//! Records mono f32 PCM via cpal on a dedicated thread (cpal streams are not
//! Send/Sync on all platforms), then resamples to 16 kHz for Whisper.

use crate::error::{AppError, AppResult};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, StreamConfig};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// One host input device (for Settings enumeration).
#[derive(Debug, Clone, serde::Serialize)]
pub struct InputDeviceInfo {
    /// Human-readable device name (also the persisted preference key).
    pub name: String,
    /// True when this is the host's current default input.
    pub is_default: bool,
}

/// Result of resolving a preferred mic name against the current host list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedInput {
    /// Preference unset/empty → use host default (no name pin).
    SystemDefault,
    /// Preference matched a live device name.
    Named(String),
    /// Preference set but not found → fall back to host default for this session.
    FallbackDefault { preferred: String },
}

impl ResolvedInput {
    /// Name to log for the device that will actually be used.
    pub fn log_label(&self) -> String {
        match self {
            Self::SystemDefault => "system default".into(),
            Self::Named(name) => name.clone(),
            Self::FallbackDefault { preferred } => {
                format!("system default (preferred {preferred:?} unavailable)")
            }
        }
    }
}

/// Structured open-time mic resolution (label for logs + structured fallback flag).
///
/// Prefer this over re-parsing free-form `device_label` strings — device names
/// may legitimately contain words like "unavailable".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicOpenInfo {
    /// Human-readable label of the device actually opened.
    pub device_label: String,
    /// When set, preferred named device was not used; value is the preferred name.
    pub preferred_unavailable: Option<String>,
}

impl MicOpenInfo {
    pub fn system_default() -> Self {
        Self {
            device_label: ResolvedInput::SystemDefault.log_label(),
            preferred_unavailable: None,
        }
    }

    pub fn from_resolved(resolved: &ResolvedInput) -> Self {
        Self {
            device_label: resolved.log_label(),
            preferred_unavailable: match resolved {
                ResolvedInput::FallbackDefault { preferred } => Some(preferred.clone()),
                ResolvedInput::SystemDefault | ResolvedInput::Named(_) => None,
            },
        }
    }

    /// Clear log/status notice when preferred was unavailable (`None` when no fallback).
    pub fn fallback_notice(&self) -> Option<String> {
        self.preferred_unavailable
            .as_ref()
            .map(|name| format!("Preferred mic {name:?} unavailable — using system default"))
    }
}

/// Resolve a persisted preference against a list of available device names.
///
/// - `None` / empty / whitespace → SystemDefault
/// - exact name match → Named
/// - otherwise → FallbackDefault (caller opens host default)
///
/// Pure function for unit tests (no cpal).
pub fn resolve_input_preference(
    preferred: Option<&str>,
    available_names: &[String],
) -> ResolvedInput {
    let name = match preferred {
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                return ResolvedInput::SystemDefault;
            }
            t
        }
        None => return ResolvedInput::SystemDefault,
    };

    if available_names.iter().any(|n| n == name) {
        ResolvedInput::Named(name.to_string())
    } else {
        ResolvedInput::FallbackDefault {
            preferred: name.to_string(),
        }
    }
}

/// Normalize settings field: empty / whitespace → None (system default).
pub fn normalize_input_device_name(name: Option<&str>) -> Option<String> {
    name.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

/// List host input devices (cpal). Does not include a synthetic "System default" row —
/// the UI adds that. Failures return an error (Settings stays usable for other prefs).
///
/// Called on Settings open and on every **Refresh** — no cache; re-enumeration is
/// the refresh path.
pub fn list_input_devices() -> AppResult<Vec<InputDeviceInfo>> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());

    let devices = host
        .input_devices()
        .map_err(|e| AppError::from(format!("Failed to enumerate microphones: {e}")))?;

    let mut out = Vec::new();
    for device in devices {
        let name = match device.name() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let is_default = default_name.as_ref() == Some(&name);
        out.push(InputDeviceInfo { name, is_default });
    }
    Ok(out)
}

/// Active capture session. `stop()` joins the audio thread.
pub struct RecordingSession {
    stop_flag: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: Arc<Mutex<u32>>,
    /// Log label from **open-time** resolution on the mic thread (what capture
    /// actually uses). Not a pre-open guess. Examples: device name,
    /// `"system default"`, or `"system default (preferred … unavailable)"`.
    pub device_label: String,
    /// Preferred name when it could not be opened (structured; not string-sniffed).
    pub preferred_unavailable: Option<String>,
    stream_failed: Arc<AtomicBool>,
    join: Option<JoinHandle<AppResult<()>>>,
}

impl RecordingSession {
    /// Start capture. `preferred` is a device name from settings (`None`/empty = system default).
    ///
    /// Resolution (name → device or host default) happens once on the mic thread
    /// at open time; [`MicOpenInfo`] reflects that outcome.
    pub fn start(preferred: Option<&str>) -> AppResult<Self> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let sample_rate = Arc::new(Mutex::new(0u32));
        let stream_failed = Arc::new(AtomicBool::new(false));
        // Structured open result — published after open_input_device, before sample rate.
        let open_info_shared = Arc::new(Mutex::new(None::<MicOpenInfo>));

        // Always pass the normalized preference through; open-time lookup decides.
        // (Do not pre-list here — that double-enumerated and dropped the name on list errors.)
        let preferred_t = normalize_input_device_name(preferred);

        let stop_flag_t = Arc::clone(&stop_flag);
        let samples_t = Arc::clone(&samples);
        let sample_rate_t = Arc::clone(&sample_rate);
        let open_info_t = Arc::clone(&open_info_shared);
        let stream_failed_t = Arc::clone(&stream_failed);

        let join = thread::Builder::new()
            .name("eaglescribe-mic".into())
            .spawn(move || {
                run_capture(
                    stop_flag_t,
                    samples_t,
                    sample_rate_t,
                    open_info_t,
                    preferred_t,
                    stream_failed_t,
                )
            })
            .map_err(|e| AppError::from(format!("Failed to spawn mic thread: {e}")))?;

        // Wait for open-time resolution first (AC6: need real label/fallback, not "pending").
        // Open publishes MicOpenInfo before sample rate; do not snapshot on rate alone.
        for _ in 0..200 {
            if open_info_shared.lock().is_some() {
                break;
            }
            if join.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        // Then briefly wait for the stream sample rate (non-critical for fallback notice).
        for _ in 0..50 {
            if *sample_rate.lock() > 0 {
                break;
            }
            if join.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        // Re-read after waits so a late-publishing open is not stuck as "pending".
        let open_info = open_info_shared.lock().clone().unwrap_or_else(|| {
            // Open still running or failed before publish — unknown outcome, not a fallback.
            match normalize_input_device_name(preferred) {
                Some(name) => MicOpenInfo {
                    device_label: format!("pending ({name})"),
                    preferred_unavailable: None,
                },
                None => MicOpenInfo::system_default(),
            }
        });

        Ok(Self {
            stop_flag,
            samples,
            sample_rate,
            device_label: open_info.device_label,
            preferred_unavailable: open_info.preferred_unavailable,
            stream_failed,
            join: Some(join),
        })
    }

    pub fn failure_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stream_failed)
    }

    /// Stop capture and return mono samples at the device sample rate.
    pub fn stop(mut self) -> AppResult<(Vec<f32>, u32)> {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            match join.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(AppError::from("Microphone thread panicked")),
            }
        }

        let rate = *self.sample_rate.lock();
        let samples = self.samples.lock().clone();
        if samples.is_empty() {
            return Err(AppError::from(
                "No audio captured — check microphone permissions",
            ));
        }
        if rate == 0 {
            return Err(AppError::from("Microphone never reported a sample rate"));
        }
        Ok((samples, rate))
    }
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            // Avoid blocking forever if the audio callback wedges.
            let _ = join.join();
        }
    }
}

/// Open preferred input (single enumeration) or host default.
/// Returns the device and structured open-time info (label + fallback flag).
fn open_input_device(
    host: &cpal::Host,
    preferred_name: Option<&str>,
) -> AppResult<(cpal::Device, MicOpenInfo)> {
    let Some(want) = preferred_name else {
        let device = host
            .default_input_device()
            .ok_or_else(|| AppError::from("No default microphone found"))?;
        return Ok((device, MicOpenInfo::system_default()));
    };

    match host.input_devices() {
        Ok(devices) => {
            // Single enumeration: resolve via the pure helper, then open the match.
            let devices: Vec<cpal::Device> = devices.collect();
            let names: Vec<String> = devices.iter().filter_map(|d| d.name().ok()).collect();
            let resolved = resolve_input_preference(Some(want), &names);
            match &resolved {
                ResolvedInput::Named(n) => {
                    for device in devices {
                        if device.name().ok().as_deref() == Some(n.as_str()) {
                            return Ok((device, MicOpenInfo::from_resolved(&resolved)));
                        }
                    }
                    // Name was in the list but device vanished mid-loop — fall through.
                    eprintln!(
                        "[eaglescribe] preferred mic {want:?} disappeared at open; using system default"
                    );
                    let device = host
                        .default_input_device()
                        .ok_or_else(|| AppError::from("No default microphone found"))?;
                    let fb = ResolvedInput::FallbackDefault {
                        preferred: want.to_string(),
                    };
                    Ok((device, MicOpenInfo::from_resolved(&fb)))
                }
                ResolvedInput::FallbackDefault { .. } => {
                    eprintln!(
                        "[eaglescribe] preferred mic {want:?} not found at open; using system default"
                    );
                    let device = host
                        .default_input_device()
                        .ok_or_else(|| AppError::from("No default microphone found"))?;
                    Ok((device, MicOpenInfo::from_resolved(&resolved)))
                }
                ResolvedInput::SystemDefault => {
                    // prefer was non-empty; treat as host default for safety.
                    let device = host
                        .default_input_device()
                        .ok_or_else(|| AppError::from("No default microphone found"))?;
                    Ok((device, MicOpenInfo::from_resolved(&resolved)))
                }
            }
        }
        Err(e) => {
            // List failed: still open host default so capture can proceed.
            // Structured flag marks preferred as not opened (do not claim host "unavailable" by name sniff).
            eprintln!(
                "[eaglescribe] failed to enumerate inputs ({e}); using system default (preferred {want:?} not opened)"
            );
            let device = host
                .default_input_device()
                .ok_or_else(|| AppError::from("No default microphone found"))?;
            Ok((
                device,
                MicOpenInfo {
                    device_label: format!(
                        "system default (input list failed; preferred {want:?} not opened)"
                    ),
                    preferred_unavailable: Some(want.to_string()),
                },
            ))
        }
    }
}

fn run_capture(
    stop_flag: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate_out: Arc<Mutex<u32>>,
    open_info_out: Arc<Mutex<Option<MicOpenInfo>>>,
    preferred_name: Option<String>,
    stream_failed: Arc<AtomicBool>,
) -> AppResult<()> {
    let host = cpal::default_host();
    let (device, info) = open_input_device(&host, preferred_name.as_deref())?;
    // Publish structured open result before sample rate so waiters see accurate fallback state.
    *open_info_out.lock() = Some(info);

    let config = device
        .default_input_config()
        .map_err(|e| AppError::from(format!("Input config error: {e}")))?;

    let sample_rate = config.sample_rate().0;
    *sample_rate_out.lock() = sample_rate;

    let channels = config.channels() as usize;
    let sample_format = config.sample_format();
    let stream_config: StreamConfig = config.clone().into();

    let stop_cb = Arc::clone(&stop_flag);
    let samples_cb = Arc::clone(&samples);
    let stream = match sample_format {
        SampleFormat::F32 => build_stream::<f32>(
            &device,
            &stream_config,
            channels,
            samples_cb,
            stop_cb,
            Arc::clone(&stream_failed),
        )?,
        SampleFormat::I16 => build_stream::<i16>(
            &device,
            &stream_config,
            channels,
            samples_cb,
            stop_cb,
            Arc::clone(&stream_failed),
        )?,
        SampleFormat::U16 => build_stream::<u16>(
            &device,
            &stream_config,
            channels,
            samples_cb,
            stop_cb,
            Arc::clone(&stream_failed),
        )?,
        other => {
            return Err(AppError::from(format!(
                "Unsupported sample format: {other:?}"
            )))
        }
    };

    stream
        .play()
        .map_err(|e| AppError::from(format!("Failed to start mic stream: {e}")))?;

    loop {
        if stream_failed.load(Ordering::SeqCst) {
            drop(stream);
            return Err(AppError::from(
                "Microphone input device was lost during recording",
            ));
        }
        if stop_flag.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    // Drop stream on this thread.
    drop(stream);
    if stream_failed.load(Ordering::SeqCst) {
        return Err(AppError::from(
            "Microphone input device was lost during recording",
        ));
    }
    Ok(())
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    samples: Arc<Mutex<Vec<f32>>>,
    stop_flag: Arc<AtomicBool>,
    stream_failed: Arc<AtomicBool>,
) -> AppResult<cpal::Stream>
where
    T: Sample + cpal::SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let err_fn = move |err| {
        stream_failed.store(true, Ordering::SeqCst);
        eprintln!("[eaglescribe] audio stream error: {err}");
    };

    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if stop_flag.load(Ordering::SeqCst) {
                    return;
                }
                let mut out = samples.lock();
                if channels <= 1 {
                    for &sample in data {
                        out.push(f32::from_sample(sample));
                    }
                } else {
                    for frame in data.chunks(channels) {
                        let mut sum = 0.0f32;
                        for &sample in frame {
                            sum += f32::from_sample(sample);
                        }
                        out.push(sum / channels as f32);
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| AppError::from(format!("Failed to build input stream: {e}")))
}

/// Whisper / trim pipeline sample rate.
pub const SAMPLE_RATE_16K: u32 = 16_000;

/// Frame length for energy detection (~20 ms). Not user-facing.
const TRIM_FRAME_MS: u32 = 20;
/// Fixed pad kept before first / after last speech frame.
///
/// Generous so soft word onsets/offsets are not shaved before Whisper. Spec
/// range is ~50–150 ms for the energy edge; we use 200 ms plus a separate
/// Whisper tail pad (see [`pad_for_whisper_16k`]).
const TRIM_PAD_MS: u32 = 200;
/// Minimum remaining audio after trim (~150 ms). Below this → fail path.
const TRIM_MIN_REMAINING_MS: u32 = 150;
/// Absolute floor for frame RMS (f32 PCM in [-1, 1]). Below this is never speech.
///
/// Dogfood last_capture.wav (peak ≈ 0.018) had real second-half words at
/// frame RMS under the old fixed 0.005 gate — trim cut 5+ s of speech and
/// Whisper never saw them. Floor stays above near-silent TCC denial (~0).
const TRIM_RMS_FLOOR: f32 = 0.0015;
/// Cap so loud takes can still drop room noise after the last word.
const TRIM_RMS_CEILING: f32 = 0.005;
/// Adaptive gate: `clamp(peak_abs * this, FLOOR, CEILING)`.
///
/// Quiet mics (low peak) get a lower threshold so soft follow-on phrases
/// count as speech; loud mics stay near the historical 0.005 ceiling.
const TRIM_RMS_RELATIVE: f32 = 0.12;
/// Pure silence appended after capture/trim so Whisper has decoder "breathing
/// room" after the last word. Without this, greedy/base models often cut the
/// end of a long or multi-sentence take.
pub const WHISPER_TAIL_PAD_MS: u32 = 400;
/// Keep the mic open this long after the user releases the hold hotkey (or hits
/// Stop) so trailing words of a second sentence are not lost when the chord is
/// lifted slightly early. Session still captures during this window.
pub const RECORDING_POST_ROLL_MS: u64 = 500;
/// Peak below this after capture is treated as a silent stream (often denied mic).
pub const SILENT_CAPTURE_PEAK: f32 = 0.001;

/// Exact production preprocessing configuration used by Recognition Fingerprints.
pub fn preprocessing_behavior_descriptor() -> String {
    format!(
        "resampler=linear;sample_rate={SAMPLE_RATE_16K};trim_frame_ms={TRIM_FRAME_MS};trim_pad_ms={TRIM_PAD_MS};trim_min_ms={TRIM_MIN_REMAINING_MS};rms_floor={TRIM_RMS_FLOOR};rms_ceiling={TRIM_RMS_CEILING};rms_relative={TRIM_RMS_RELATIVE};tail_pad_ms={WHISPER_TAIL_PAD_MS};silent_peak={SILENT_CAPTURE_PEAK}"
    )
}

/// Peak absolute sample in a mono buffer (for diagnostics / silent-stream detect).
pub fn peak_abs(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max)
}

/// Default / floor frame-RMS gate (tests / diagnostics). Prefer
/// [`speech_rms_threshold`] with the actual buffer when logging a take.
pub fn trim_rms_threshold() -> f32 {
    TRIM_RMS_FLOOR
}

/// Adaptive speech gate for a mono buffer: scales with peak level so quiet
/// laptop mics do not lose the second half of an utterance.
pub fn speech_rms_threshold(samples: &[f32]) -> f32 {
    let peak = peak_abs(samples);
    if peak <= 0.0 {
        return TRIM_RMS_FLOOR;
    }
    (peak * TRIM_RMS_RELATIVE).clamp(TRIM_RMS_FLOOR, TRIM_RMS_CEILING)
}

/// Successful leading/trailing silence trim of 16 kHz mono PCM.
#[derive(Debug, Clone, PartialEq)]
pub struct TrimResult {
    pub samples: Vec<f32>,
    /// Original buffer duration in milliseconds.
    pub original_ms: u32,
    /// Post-trim duration in milliseconds.
    pub trimmed_ms: u32,
    /// Silence removed from the head (ms).
    pub head_ms: u32,
    /// Silence removed from the tail (ms).
    pub tail_ms: u32,
}

/// Outcome of [`trim_silence_16k`].
#[derive(Debug, Clone, PartialEq)]
pub enum TrimOutcome {
    Ok(TrimResult),
    /// Remaining audio shorter than the min floor (or empty / all silence).
    BelowFloor {
        original_ms: u32,
        remaining_ms: u32,
    },
}

fn samples_to_ms(len: usize) -> u32 {
    if len == 0 {
        return 0;
    }
    ((len as u64 * 1000) / SAMPLE_RATE_16K as u64) as u32
}

fn ms_to_samples(ms: u32) -> usize {
    (ms as u64 * SAMPLE_RATE_16K as u64 / 1000) as usize
}

fn frame_rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = frame.iter().map(|s| s * s).sum();
    (sum_sq / frame.len() as f32).sqrt()
}

/// Leading/trailing energy (RMS) silence trim on **16 kHz mono** PCM.
///
/// Mid-utterance quiet frames are never removed — only head/tail padding
/// outside the first and last speech frames (with a fixed edge pad).
///
/// Speech frames use an **adaptive** RMS gate ([`speech_rms_threshold`]) so
/// quieter second phrases on a soft mic are not treated as trailing silence.
///
/// Pure function: no mic I/O. Unit-test with synthetic buffers.
pub fn trim_silence_16k(samples: &[f32]) -> TrimOutcome {
    let original_ms = samples_to_ms(samples.len());
    if samples.is_empty() {
        return TrimOutcome::BelowFloor {
            original_ms: 0,
            remaining_ms: 0,
        };
    }

    let frame_len = ms_to_samples(TRIM_FRAME_MS).max(1);
    let pad_samples = ms_to_samples(TRIM_PAD_MS);
    let min_remaining = ms_to_samples(TRIM_MIN_REMAINING_MS);
    let threshold = speech_rms_threshold(samples);

    // Classify frames; last partial frame is included.
    let n_frames = samples.len().div_ceil(frame_len);
    let mut first_speech: Option<usize> = None;
    let mut last_speech: Option<usize> = None;

    for fi in 0..n_frames {
        let start = fi * frame_len;
        let end = (start + frame_len).min(samples.len());
        let rms = frame_rms(&samples[start..end]);
        if rms >= threshold {
            if first_speech.is_none() {
                first_speech = Some(fi);
            }
            last_speech = Some(fi);
        }
    }

    let (Some(first_fi), Some(last_fi)) = (first_speech, last_speech) else {
        return TrimOutcome::BelowFloor {
            original_ms,
            remaining_ms: 0,
        };
    };

    // Speech span in samples, then expand by pad (clamped to buffer).
    let speech_start = first_fi * frame_len;
    let speech_end = ((last_fi + 1) * frame_len).min(samples.len());
    let start = speech_start.saturating_sub(pad_samples);
    let end = (speech_end + pad_samples).min(samples.len());

    if end <= start || end - start < min_remaining {
        return TrimOutcome::BelowFloor {
            original_ms,
            remaining_ms: samples_to_ms(end.saturating_sub(start)),
        };
    }

    let trimmed = samples[start..end].to_vec();
    let head_ms = samples_to_ms(start);
    let tail_ms = samples_to_ms(samples.len() - end);
    let trimmed_ms = samples_to_ms(trimmed.len());

    TrimOutcome::Ok(TrimResult {
        samples: trimmed,
        original_ms,
        trimmed_ms,
        head_ms,
        tail_ms,
    })
}

/// Append trailing silence so Whisper does not clip the last words of a take.
///
/// Call after optional silence trim and before `transcribe_16k_mono`. Does not
/// change sample rate. Empty input stays empty.
pub fn pad_for_whisper_16k(samples: &[f32]) -> Vec<f32> {
    pad_silence_16k(samples, 0, WHISPER_TAIL_PAD_MS)
}

/// Prepend/append pure silence (zeros) on **16 kHz mono** PCM.
pub fn pad_silence_16k(samples: &[f32], head_ms: u32, tail_ms: u32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let head = ms_to_samples(head_ms);
    let tail = ms_to_samples(tail_ms);
    let mut out = Vec::with_capacity(samples.len() + head + tail);
    out.resize(head, 0.0);
    out.extend_from_slice(samples);
    out.resize(out.len() + tail, 0.0);
    out
}

/// Write mono f32 PCM as a 16-bit PCM WAV at 16 kHz (for last-capture debug).
pub fn write_wav_16k_mono(path: &std::path::Path, samples: &[f32]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create wav dir: {e}"))?;
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE_16K,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| format!("create wav: {e}"))?;
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let i = (clamped * i16::MAX as f32).round() as i16;
        writer
            .write_sample(i)
            .map_err(|e| format!("write sample: {e}"))?;
    }
    writer.finalize().map_err(|e| format!("finalize wav: {e}"))?;
    Ok(())
}

/// Default path for the most recent capture dump (`last_capture.wav`).
pub fn default_last_capture_path() -> std::path::PathBuf {
    if let Some(data) = dirs::data_local_dir() {
        return data.join("eaglescribe").join("last_capture.wav");
    }
    std::path::PathBuf::from("last_capture.wav")
}

/// Linear resample mono audio to 16 kHz (Whisper input).
pub fn resample_to_16k(samples: &[f32], src_rate: u32) -> Vec<f32> {
    const TARGET: u32 = SAMPLE_RATE_16K;
    if src_rate == TARGET || samples.is_empty() {
        return samples.to_vec();
    }

    let ratio = src_rate as f64 / TARGET as f64;
    let out_len = ((samples.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = samples[idx];
        let b = samples.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn resolve_none_is_system_default() {
        assert_eq!(
            resolve_input_preference(None, &names(&["Built-in Microphone"])),
            ResolvedInput::SystemDefault
        );
    }

    #[test]
    fn resolve_empty_is_system_default() {
        assert_eq!(
            resolve_input_preference(Some(""), &names(&["Mic A"])),
            ResolvedInput::SystemDefault
        );
        assert_eq!(
            resolve_input_preference(Some("   "), &names(&["Mic A"])),
            ResolvedInput::SystemDefault
        );
    }

    #[test]
    fn resolve_exact_match() {
        let avail = names(&["Built-in Microphone", "USB Headset"]);
        assert_eq!(
            resolve_input_preference(Some("USB Headset"), &avail),
            ResolvedInput::Named("USB Headset".into())
        );
    }

    #[test]
    fn resolve_missing_falls_back() {
        let avail = names(&["Built-in Microphone"]);
        assert_eq!(
            resolve_input_preference(Some("USB Headset"), &avail),
            ResolvedInput::FallbackDefault {
                preferred: "USB Headset".into()
            }
        );
    }

    #[test]
    fn resolve_missing_with_empty_list_falls_back() {
        assert_eq!(
            resolve_input_preference(Some("Gone"), &[]),
            ResolvedInput::FallbackDefault {
                preferred: "Gone".into()
            }
        );
    }

    #[test]
    fn normalize_input_device_name_empty_to_none() {
        assert_eq!(normalize_input_device_name(None), None);
        assert_eq!(normalize_input_device_name(Some("")), None);
        assert_eq!(normalize_input_device_name(Some("  ")), None);
        assert_eq!(
            normalize_input_device_name(Some("USB Mic")),
            Some("USB Mic".into())
        );
    }

    #[test]
    fn log_label_describes_resolution() {
        assert_eq!(ResolvedInput::SystemDefault.log_label(), "system default");
        assert_eq!(ResolvedInput::Named("USB".into()).log_label(), "USB");
        let fb = ResolvedInput::FallbackDefault {
            preferred: "X".into(),
        };
        let label = fb.log_label();
        assert!(label.contains("unavailable"), "{label}");
        assert!(label.contains("system default"), "{label}");
        assert!(label.contains("X"), "{label}");
        assert!(matches!(fb, ResolvedInput::FallbackDefault { .. }));
        assert!(matches!(
            ResolvedInput::SystemDefault,
            ResolvedInput::SystemDefault
        ));
        assert!(matches!(
            ResolvedInput::Named("USB".into()),
            ResolvedInput::Named(_)
        ));
    }

    #[test]
    fn mic_open_info_fallback_notice_is_structured() {
        let info = MicOpenInfo::from_resolved(&ResolvedInput::FallbackDefault {
            preferred: "USB Headset".into(),
        });
        assert_eq!(info.preferred_unavailable.as_deref(), Some("USB Headset"));
        assert_eq!(
            info.fallback_notice().as_deref(),
            Some("Preferred mic \"USB Headset\" unavailable — using system default")
        );

        // Named open with awkward device name must NOT be treated as fallback.
        let named =
            MicOpenInfo::from_resolved(&ResolvedInput::Named("Mic unavailable for studio".into()));
        assert!(named.preferred_unavailable.is_none());
        assert!(named.fallback_notice().is_none());
        assert_eq!(named.device_label, "Mic unavailable for studio");

        assert!(MicOpenInfo::system_default().fallback_notice().is_none());

        // List-failure path sets preferred_unavailable without string-sniffing the label.
        let list_fail = MicOpenInfo {
            device_label: "system default (input list failed; preferred \"X\" not opened)".into(),
            preferred_unavailable: Some("X".into()),
        };
        assert_eq!(
            list_fail.fallback_notice().as_deref(),
            Some("Preferred mic \"X\" unavailable — using system default")
        );
    }

    #[test]
    fn resolve_missing_log_label_is_clear_for_ui() {
        // Acceptance: missing preferred → fallback label is explicit (no silent surprise).
        let avail = names(&["Built-in Microphone"]);
        let r = resolve_input_preference(Some("USB Headset"), &avail);
        assert!(matches!(
            &r,
            ResolvedInput::FallbackDefault { preferred } if preferred == "USB Headset"
        ));
        let info = MicOpenInfo::from_resolved(&r);
        assert!(info.device_label.contains("unavailable"));
        assert_eq!(
            info.fallback_notice().as_deref(),
            Some("Preferred mic \"USB Headset\" unavailable — using system default")
        );
    }

    #[test]
    fn resample_identity_at_16k() {
        let s = vec![0.0, 0.5, 1.0];
        assert_eq!(resample_to_16k(&s, 16_000), s);
    }

    /// Build 16 kHz mono: `silence_ms` of zeros, then a speech burst of
    /// `speech_ms` (sine at amplitude 0.3), then trailing silence.
    fn synth_pad_speech_pad(head_ms: u32, speech_ms: u32, tail_ms: u32) -> Vec<f32> {
        let mut out = vec![0.0f32; ms_to_samples(head_ms)];
        let speech_n = ms_to_samples(speech_ms);
        let freq = 440.0f32;
        for i in 0..speech_n {
            let t = i as f32 / SAMPLE_RATE_16K as f32;
            out.push(0.3 * (2.0 * std::f32::consts::PI * freq * t).sin());
        }
        out.extend(std::iter::repeat_n(0.0f32, ms_to_samples(tail_ms)));
        out
    }

    /// Speech — long quiet mid — speech (mid pause must be preserved).
    fn synth_speech_pause_speech(a_ms: u32, pause_ms: u32, b_ms: u32) -> Vec<f32> {
        let mut out = synth_pad_speech_pad(0, a_ms, 0);
        out.extend(std::iter::repeat_n(0.0f32, ms_to_samples(pause_ms)));
        out.extend(synth_pad_speech_pad(0, b_ms, 0));
        out
    }

    #[test]
    fn trim_empty_is_below_floor() {
        assert_eq!(
            trim_silence_16k(&[]),
            TrimOutcome::BelowFloor {
                original_ms: 0,
                remaining_ms: 0
            }
        );
    }

    #[test]
    fn trim_all_silence_is_below_floor() {
        let s = vec![0.0f32; ms_to_samples(2000)];
        match trim_silence_16k(&s) {
            TrimOutcome::BelowFloor {
                original_ms,
                remaining_ms,
            } => {
                assert!((1990..=2010).contains(&original_ms), "{original_ms}");
                assert_eq!(remaining_ms, 0);
            }
            other => panic!("expected BelowFloor, got {other:?}"),
        }
    }

    #[test]
    fn trim_near_silent_noise_is_below_floor() {
        // Below RMS floor — still treated as silence by the trimmer itself.
        // (Pipeline may still soft-recover non-zero peak into Whisper.)
        let s: Vec<f32> = (0..ms_to_samples(500)).map(|_| 0.0005).collect();
        assert!(matches!(
            trim_silence_16k(&s),
            TrimOutcome::BelowFloor { .. }
        ));
        assert!(peak_abs(&s) < TRIM_RMS_FLOOR);
    }

    #[test]
    fn quiet_speech_above_lowered_threshold_is_kept() {
        // Soft speech at amplitude 0.02 must survive trim (was failing at 0.015).
        let mut s = vec![0.0f32; ms_to_samples(300)];
        let speech = ms_to_samples(400);
        for i in 0..speech {
            let t = i as f32 / SAMPLE_RATE_16K as f32;
            s.push(0.02 * (2.0 * std::f32::consts::PI * 440.0 * t).sin());
        }
        s.extend(std::iter::repeat_n(0.0f32, ms_to_samples(200)));
        assert!(
            matches!(trim_silence_16k(&s), TrimOutcome::Ok(_)),
            "0.02 amplitude speech must not be fully trimmed"
        );
    }

    #[test]
    fn trim_keeps_quieter_second_half_of_utterance() {
        // Regression: dogfood last_capture — louder first phrase, softer second
        // half (still real speech). Old fixed 0.005 gate dropped the second half.
        // Peak ~0.018, second half ~0.01 peak → frame RMS often < 0.005.
        let mut s = vec![0.0f32; ms_to_samples(500)];
        // Louder first phrase (~1.5 s).
        for i in 0..ms_to_samples(1500) {
            let t = i as f32 / SAMPLE_RATE_16K as f32;
            s.push(0.018 * (2.0 * std::f32::consts::PI * 220.0 * t).sin());
        }
        // Brief dip (not long enough to matter — mid frames are kept once span is set).
        s.extend(std::iter::repeat_n(0.0f32, ms_to_samples(200)));
        // Quieter second phrase (~3 s) that the old gate treated as silence.
        let second_start = s.len();
        for i in 0..ms_to_samples(3000) {
            let t = i as f32 / SAMPLE_RATE_16K as f32;
            s.push(0.008 * (2.0 * std::f32::consts::PI * 330.0 * t).sin());
        }
        let second_end = s.len();
        s.extend(std::iter::repeat_n(0.0f32, ms_to_samples(800)));

        let out = match trim_silence_16k(&s) {
            TrimOutcome::Ok(r) => r,
            other => panic!("expected Ok, got {other:?}"),
        };
        // Trimmed buffer must include samples from the quiet second phrase.
        // Map second phrase into the trimmed window via original indices.
        let head_samples = ms_to_samples(out.head_ms);
        let keep_start = head_samples;
        let keep_end = keep_start + out.samples.len();
        assert!(
            keep_start < second_end && keep_end > second_start,
            "quiet second half must not be fully tail-trimmed; head_ms={} trimmed_ms={} (second span samples {second_start}..{second_end}, kept {keep_start}..{keep_end})",
            out.head_ms,
            out.trimmed_ms
        );
        // Should keep most of the ~5.2 s of speech content (not collapse to ~1.5 s).
        assert!(
            out.trimmed_ms >= 4000,
            "expected multi-second keep, got {}ms",
            out.trimmed_ms
        );
    }

    #[test]
    fn trim_leading_silence_removes_head() {
        // ≥1 s silence, then 400 ms speech, short tail.
        let s = synth_pad_speech_pad(1200, 400, 50);
        let out = match trim_silence_16k(&s) {
            TrimOutcome::Ok(r) => r,
            other => panic!("expected Ok, got {other:?}"),
        };
        assert!(
            out.head_ms >= 1000,
            "expected ≥1s head removed, got {}ms",
            out.head_ms
        );
        assert!(out.trimmed_ms < out.original_ms);
        // Speech + pad should remain (~400 + pad on each side, minus short tail).
        assert!(
            out.trimmed_ms >= 400 && out.trimmed_ms < 900,
            "trimmed_ms={}",
            out.trimmed_ms
        );
        // Trimmed buffer must contain non-silent samples.
        let peak = out.samples.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
        assert!(peak > 0.1, "speech energy missing after trim");
    }

    #[test]
    fn trim_trailing_silence_removes_tail() {
        let s = synth_pad_speech_pad(50, 400, 1200);
        let out = match trim_silence_16k(&s) {
            TrimOutcome::Ok(r) => r,
            other => panic!("expected Ok, got {other:?}"),
        };
        // 1200 ms trailing silence minus ~TRIM_PAD_MS edge pad (frame-rounded).
        assert!(
            out.tail_ms >= 900,
            "expected ~1s tail removed, got {}ms",
            out.tail_ms
        );
        assert!(out.trimmed_ms < out.original_ms);
        assert!(out.trimmed_ms >= 400);
    }

    #[test]
    fn trim_both_ends_keeps_speech_and_pad() {
        let s = synth_pad_speech_pad(1000, 500, 1000);
        let out = match trim_silence_16k(&s) {
            TrimOutcome::Ok(r) => r,
            other => panic!("expected Ok, got {other:?}"),
        };
        assert!(out.head_ms >= 700, "head_ms={}", out.head_ms);
        assert!(out.tail_ms >= 700, "tail_ms={}", out.tail_ms);
        // 500 ms speech + pad each side (TRIM_PAD_MS) ≈ 900 ms, not full 2500.
        assert!(
            out.trimmed_ms >= 500 && out.trimmed_ms <= 1100,
            "trimmed_ms={}",
            out.trimmed_ms
        );
        // Edge pad: we should not cut exactly at first non-zero sample.
        // First sample of the original speech block is after 1000 ms head.
        // With pad, head_ms should be roughly 1000 - PAD, not 1000.
        assert!(
            out.head_ms < 1000,
            "pad should leave some pre-roll; head_ms={}",
            out.head_ms
        );
    }

    #[test]
    fn trim_preserves_mid_utterance_pause() {
        // 300 ms speech, 800 ms silence, 300 ms speech — mid pause stays.
        let s = synth_speech_pause_speech(300, 800, 300);
        let out = match trim_silence_16k(&s) {
            TrimOutcome::Ok(r) => r,
            other => panic!("expected Ok, got {other:?}"),
        };
        // Full span ≈ 300+800+300 = 1400 ms (+ pad if any outer silence).
        assert!(
            out.trimmed_ms >= 1300,
            "mid pause must not be collapsed; trimmed_ms={}",
            out.trimmed_ms
        );
        // Original has no outer pad, so head/tail removed should be small (≤ pad).
        assert!(out.head_ms <= TRIM_PAD_MS + TRIM_FRAME_MS);
        assert!(out.tail_ms <= TRIM_PAD_MS + TRIM_FRAME_MS);
    }

    #[test]
    fn trim_speech_at_buffer_start_keeps_onset_pad_or_start() {
        // Speech immediately — no leading silence to strip beyond pad clamp.
        let s = synth_pad_speech_pad(0, 400, 800);
        let out = match trim_silence_16k(&s) {
            TrimOutcome::Ok(r) => r,
            other => panic!("expected Ok, got {other:?}"),
        };
        // Head remove is 0 when speech starts at sample 0 (pad clamps to 0).
        assert_eq!(out.head_ms, 0);
        // First samples of speech present (not clipped away by hard zero cut).
        let early_peak = out
            .samples
            .iter()
            .take(ms_to_samples(50))
            .map(|x| x.abs())
            .fold(0.0f32, f32::max);
        assert!(
            early_peak > 0.05,
            "onset should not be systematically zeroed"
        );
    }

    #[test]
    fn trim_very_short_speech_below_floor() {
        // A single quiet-ish frame of "speech" that ends up below min remaining
        // after bounds — use a tiny burst shorter than min floor with no room for pad.
        let short = synth_pad_speech_pad(500, 40, 500);
        // 40 ms speech + pad on each side may still exceed floor depending on
        // detection; force BelowFloor with only sub-threshold samples at edges
        // and a speech burst shorter than min when pad cannot help enough.
        // Use a near-threshold short spike: 20 ms at high energy, rest silence.
        let mut s = vec![0.0f32; ms_to_samples(1000)];
        let spike_start = ms_to_samples(500);
        let spike_end = spike_start + ms_to_samples(20);
        for sample in &mut s[spike_start..spike_end] {
            *sample = 0.5;
        }
        // 20 ms + 2*100 ms pad = 220 ms > 150 ms floor — so this should Ok.
        // Use even shorter: 1 sample of speech after long silence → pad gives ~200ms.
        // To hit BelowFloor with speech detected: need speech span + pad < min.
        // With pad=100ms each side, min=150ms, almost any single speech frame
        // (20ms) + pad exceeds floor. So BelowFloor with speech is rare unless
        // buffer itself is tiny.
        let tiny = vec![0.5f32; ms_to_samples(40)]; // 40 ms of speech only
        match trim_silence_16k(&tiny) {
            TrimOutcome::BelowFloor { remaining_ms, .. } => {
                assert!(remaining_ms < TRIM_MIN_REMAINING_MS);
            }
            TrimOutcome::Ok(r) => {
                // If pad clamps to full buffer and buffer < min → BelowFloor only.
                // 40 ms buffer always < 150 ms floor.
                panic!("expected BelowFloor for 40ms buffer, got Ok {r:?}");
            }
        }
        // Ensure the 20 ms spike with room for pad still succeeds (sanity).
        assert!(
            matches!(trim_silence_16k(&s), TrimOutcome::Ok(_)),
            "20ms speech with pad room should pass floor"
        );
        let _ = short; // keep helper exercised via other tests
    }

    #[test]
    fn write_wav_16k_mono_roundtrips_peak() {
        let mut samples = vec![0.0f32; ms_to_samples(50)];
        samples[10] = 0.5;
        let dir = std::env::temp_dir().join(format!(
            "eaglescribe-wav-test-{}",
            std::process::id()
        ));
        let path = dir.join("t.wav");
        write_wav_16k_mono(&path, &samples).expect("write");
        assert!(path.is_file());
        let meta = std::fs::metadata(&path).expect("meta");
        assert!(meta.len() > 44, "wav should be larger than header");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pad_for_whisper_appends_tail_silence() {
        let speech = vec![0.25f32; ms_to_samples(200)];
        let padded = pad_for_whisper_16k(&speech);
        assert_eq!(
            padded.len(),
            speech.len() + ms_to_samples(WHISPER_TAIL_PAD_MS)
        );
        assert!(padded[..speech.len()].iter().all(|&s| s == 0.25));
        assert!(padded[speech.len()..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn pad_silence_empty_stays_empty() {
        assert!(pad_for_whisper_16k(&[]).is_empty());
        assert!(pad_silence_16k(&[], 100, 100).is_empty());
    }

    #[test]
    fn pad_silence_head_and_tail() {
        let mid = vec![1.0f32; 10];
        let out = pad_silence_16k(&mid, 0, 0);
        assert_eq!(out, mid);
        let out = pad_silence_16k(&mid, 50, 25);
        assert_eq!(out.len(), 10 + ms_to_samples(50) + ms_to_samples(25));
        assert!(out[..ms_to_samples(50)].iter().all(|&s| s == 0.0));
        assert!(out[out.len() - ms_to_samples(25)..]
            .iter()
            .all(|&s| s == 0.0));
    }

    #[test]
    fn trim_logs_fields_present_on_success() {
        let s = synth_pad_speech_pad(1000, 400, 1000);
        let out = match trim_silence_16k(&s) {
            TrimOutcome::Ok(r) => r,
            other => panic!("{other:?}"),
        };
        assert!(out.original_ms > out.trimmed_ms);
        assert!(out.head_ms > 0);
        assert!(out.tail_ms > 0);
        // samples length ↔ trimmed_ms consistency.
        assert_eq!(samples_to_ms(out.samples.len()), out.trimmed_ms);
    }

    #[test]
    fn trim_does_not_shorten_when_speech_fills_buffer() {
        // Continuous speech: head/tail removed should be ~0 (maybe a few quiet
        // edge frames if sine crosses low RMS, but not large pads of silence).
        let s = synth_pad_speech_pad(0, 1000, 0);
        let out = match trim_silence_16k(&s) {
            TrimOutcome::Ok(r) => r,
            other => panic!("{other:?}"),
        };
        assert!(
            out.head_ms + out.tail_ms < 100,
            "continuous speech should not lose large head/tail: head={} tail={}",
            out.head_ms,
            out.tail_ms
        );
        assert!(out.trimmed_ms >= 900);
    }
}
