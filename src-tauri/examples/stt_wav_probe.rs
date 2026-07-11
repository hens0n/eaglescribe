//! Offline STT probe: transcribe a 16 kHz mono WAV and print the text.
//!
//! Usage (from src-tauri):
//!   cargo run --example stt_wav_probe -- /path/to/audio.wav
//!
//! Optional second arg: model path (else uses resolve_model_path).

use eaglescribe_lib::stt::{self, WhisperEngine};
use std::env;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    let wav_path = args
        .next()
        .expect("usage: stt_wav_probe <16k-mono.wav> [model]");
    let model = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| stt::resolve_model_path(None));

    eprintln!("model: {}", model.display());
    eprintln!("wav:   {wav_path}");

    let samples = load_wav_f32_mono_16k(&wav_path).expect("load wav");
    let duration = samples.len() as f32 / 16_000.0;
    eprintln!("audio: {duration:.2}s ({} samples)", samples.len());

    // Match production pipeline: trailing pad for decoder room.
    let padded = eaglescribe_lib::audio::pad_for_whisper_16k(&samples);
    eprintln!(
        "padded: {:.2}s (+{} samples)",
        padded.len() as f32 / 16_000.0,
        padded.len().saturating_sub(samples.len())
    );

    let engine = WhisperEngine::load(&model).expect("load model");
    let text = engine.transcribe_16k_mono(&padded).expect("transcribe");
    println!("{text}");
}

/// Minimal PCM16/f32 WAV reader (mono, any rate → resampled to 16 kHz).
fn load_wav_f32_mono_16k(path: &str) -> Result<Vec<f32>, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if buf.len() < 44 || &buf[0..4] != b"RIFF" || &buf[8..12] != b"WAVE" {
        return Err("not a WAVE file".into());
    }

    let mut pos = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // format, channels, rate, bits
    let mut data_range: Option<(usize, usize)> = None;

    while pos + 8 <= buf.len() {
        let id = &buf[pos..pos + 4];
        let size = u32::from_le_bytes(buf[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let start = pos + 8;
        let end = start + size;
        if end > buf.len() {
            break;
        }
        if id == b"fmt " && size >= 16 {
            let format = u16::from_le_bytes(buf[start..start + 2].try_into().unwrap());
            let channels = u16::from_le_bytes(buf[start + 2..start + 4].try_into().unwrap());
            let rate = u32::from_le_bytes(buf[start + 4..start + 8].try_into().unwrap());
            let bits = u16::from_le_bytes(buf[start + 14..start + 16].try_into().unwrap());
            fmt = Some((format, channels, rate, bits));
        } else if id == b"data" {
            data_range = Some((start, end));
        }
        pos = end + (size % 2); // word align
    }

    let (format, channels, rate, bits) = fmt.ok_or("missing fmt")?;
    let (d0, d1) = data_range.ok_or("missing data")?;
    let data = &buf[d0..d1];

    let mut mono: Vec<f32> = Vec::new();
    match (format, bits) {
        (1, 16) => {
            let frame = channels as usize;
            let n = data.len() / 2 / frame;
            for i in 0..n {
                let mut sum = 0.0f32;
                for ch in 0..frame {
                    let off = (i * frame + ch) * 2;
                    let s = i16::from_le_bytes(data[off..off + 2].try_into().unwrap());
                    sum += s as f32 / 32768.0;
                }
                mono.push(sum / frame as f32);
            }
        }
        (3, 32) => {
            let frame = channels as usize;
            let n = data.len() / 4 / frame;
            for i in 0..n {
                let mut sum = 0.0f32;
                for ch in 0..frame {
                    let off = (i * frame + ch) * 4;
                    let s = f32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                    sum += s;
                }
                mono.push(sum / frame as f32);
            }
        }
        other => return Err(format!("unsupported wav format {other:?}")),
    }

    Ok(eaglescribe_lib::audio::resample_to_16k(&mono, rate))
}
