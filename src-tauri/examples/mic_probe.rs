//! Mic level probe for diagnosing silence-trim false failures.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .expect("no default microphone");
    let name = device.name().unwrap_or_else(|_| "?".into());
    let config = device.default_input_config().expect("input config");
    println!("device={name}");
    println!(
        "sample_rate={} channels={} format={:?}",
        config.sample_rate().0,
        config.channels(),
        config.sample_format()
    );

    let peak = Arc::new(Mutex::new(0.0f32));
    let sum_sq = Arc::new(Mutex::new(0.0f64));
    let n = Arc::new(Mutex::new(0u64));
    let peak_c = Arc::clone(&peak);
    let sum_c = Arc::clone(&sum_sq);
    let n_c = Arc::clone(&n);
    let channels = config.channels() as usize;
    let stream_config: cpal::StreamConfig = config.clone().into();
    let format = config.sample_format();

    let err_fn = |e| eprintln!("stream error: {e}");

    let stream = match format {
        cpal::SampleFormat::F32 => {
            let peak_c = Arc::clone(&peak_c);
            let sum_c = Arc::clone(&sum_c);
            let n_c = Arc::clone(&n_c);
            device
                .build_input_stream(
                    &stream_config,
                    move |data: &[f32], _| {
                        push_frames(data, channels, &peak_c, &sum_c, &n_c);
                    },
                    err_fn,
                    None,
                )
                .expect("f32 stream")
        }
        cpal::SampleFormat::I16 => {
            let peak_c = Arc::clone(&peak_c);
            let sum_c = Arc::clone(&sum_c);
            let n_c = Arc::clone(&n_c);
            device
                .build_input_stream(
                    &stream_config,
                    move |data: &[i16], _| {
                        let f: Vec<f32> = data
                            .iter()
                            .map(|&s| s as f32 / i16::MAX as f32)
                            .collect();
                        push_frames(&f, channels, &peak_c, &sum_c, &n_c);
                    },
                    err_fn,
                    None,
                )
                .expect("i16 stream")
        }
        other => panic!("unsupported format {other:?}"),
    };

    stream.play().expect("play");
    println!("Speak for 2 seconds...");
    std::thread::sleep(Duration::from_secs(2));
    drop(stream);

    let peak_v = *peak.lock().unwrap();
    let n_v = *n.lock().unwrap();
    let rms = if n_v > 0 {
        (*sum_sq.lock().unwrap() / n_v as f64).sqrt()
    } else {
        0.0
    };
    println!("peak={peak_v:.6} rms={rms:.6} mono_frames={n_v}");
    println!("trim_threshold=0.015  above_threshold={}", peak_v >= 0.015);
}

fn push_frames(
    data: &[f32],
    channels: usize,
    peak: &Mutex<f32>,
    sum_sq: &Mutex<f64>,
    n: &Mutex<u64>,
) {
    let mut p = peak.lock().unwrap();
    let mut s = sum_sq.lock().unwrap();
    let mut count = n.lock().unwrap();
    if channels <= 1 {
        for &sample in data {
            let a = sample.abs();
            *p = (*p).max(a);
            *s += (sample as f64) * (sample as f64);
            *count += 1;
        }
    } else {
        for frame in data.chunks(channels) {
            let mut sum = 0.0f32;
            for &sample in frame {
                sum += sample;
            }
            let m = sum / channels as f32;
            let a = m.abs();
            *p = (*p).max(a);
            *s += (m as f64) * (m as f64);
            *count += 1;
        }
    }
}
