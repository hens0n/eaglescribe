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

/// Active capture session. `stop()` joins the audio thread.
pub struct RecordingSession {
    stop_flag: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: Arc<Mutex<u32>>,
    join: Option<JoinHandle<AppResult<()>>>,
}

impl RecordingSession {
    pub fn start() -> AppResult<Self> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let sample_rate = Arc::new(Mutex::new(0u32));

        let stop_flag_t = Arc::clone(&stop_flag);
        let samples_t = Arc::clone(&samples);
        let sample_rate_t = Arc::clone(&sample_rate);

        let join = thread::Builder::new()
            .name("talontype-mic".into())
            .spawn(move || run_capture(stop_flag_t, samples_t, sample_rate_t))
            .map_err(|e| AppError::from(format!("Failed to spawn mic thread: {e}")))?;

        // Wait briefly for the stream to come up and report a sample rate.
        for _ in 0..50 {
            if *sample_rate.lock() > 0 {
                break;
            }
            if join.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        Ok(Self {
            stop_flag,
            samples,
            sample_rate,
            join: Some(join),
        })
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

fn run_capture(
    stop_flag: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate_out: Arc<Mutex<u32>>,
) -> AppResult<()> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| AppError::from("No default microphone found"))?;

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
        SampleFormat::F32 => {
            build_stream::<f32>(&device, &stream_config, channels, samples_cb, stop_cb)?
        }
        SampleFormat::I16 => {
            build_stream::<i16>(&device, &stream_config, channels, samples_cb, stop_cb)?
        }
        SampleFormat::U16 => {
            build_stream::<u16>(&device, &stream_config, channels, samples_cb, stop_cb)?
        }
        other => {
            return Err(AppError::from(format!(
                "Unsupported sample format: {other:?}"
            )))
        }
    };

    stream
        .play()
        .map_err(|e| AppError::from(format!("Failed to start mic stream: {e}")))?;

    while !stop_flag.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(20));
    }

    // Drop stream on this thread.
    drop(stream);
    Ok(())
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    samples: Arc<Mutex<Vec<f32>>>,
    stop_flag: Arc<AtomicBool>,
) -> AppResult<cpal::Stream>
where
    T: Sample + cpal::SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let err_fn = |err| eprintln!("[talontype] audio stream error: {err}");

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

/// Linear resample mono audio to 16 kHz (Whisper input).
pub fn resample_to_16k(samples: &[f32], src_rate: u32) -> Vec<f32> {
    const TARGET: u32 = 16_000;
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
