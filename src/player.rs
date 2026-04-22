use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub fn apply_volume(samples: &mut [f32], volume: f32, muted: bool) {
    if muted {
        samples.iter_mut().for_each(|s| *s = 0.0);
    } else {
        samples.iter_mut().for_each(|s| *s *= volume);
    }
}

pub fn start_audio_output(
    audio_buf: Arc<Mutex<VecDeque<f32>>>,
    is_paused: Arc<AtomicBool>,
    volume: Arc<Mutex<f32>>,
    is_muted: Arc<AtomicBool>,
) -> Result<cpal::Stream> {
    use crate::stream::{OUTPUT_CHANNELS, OUTPUT_SAMPLE_RATE};

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no audio output device found"))?;

    // Pin the device to the exact rate + channel count that the stream
    // thread resamples to. Without this, the device runs at its default
    // (usually 48 kHz on Linux) while the producer supplies samples at
    // whatever rate symphonia happens to emit (22 kHz for HE-AAC core),
    // and the rate mismatch manifests as continuous drift / stutter.
    let config = cpal::StreamConfig {
        channels: OUTPUT_CHANNELS as u16,
        sample_rate: cpal::SampleRate(OUTPUT_SAMPLE_RATE),
        buffer_size: cpal::BufferSize::Default,
    };
    log::info!(
        "cpal output: {} Hz, {} channels",
        OUTPUT_SAMPLE_RATE,
        OUTPUT_CHANNELS
    );

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _| {
            let paused = is_paused.load(Ordering::Relaxed);
            let muted = is_muted.load(Ordering::Relaxed);
            let vol = *volume.lock().unwrap();
            let mut buf = audio_buf.lock().unwrap();
            for sample in data.iter_mut() {
                *sample = if paused {
                    0.0
                } else {
                    buf.pop_front().unwrap_or(0.0)
                };
            }
            drop(buf);
            apply_volume(data, vol, muted);
        },
        |err| log::error!("cpal stream error: {}", err),
        None,
    )?;
    stream.play()?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_volume_scales_samples() {
        let mut s = vec![1.0f32, -1.0, 0.5];
        apply_volume(&mut s, 0.5, false);
        assert!((s[0] - 0.5).abs() < 1e-6);
        assert!((s[1] + 0.5).abs() < 1e-6);
        assert!((s[2] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn apply_volume_zeroes_when_muted() {
        let mut s = vec![1.0f32, -0.5];
        apply_volume(&mut s, 0.8, true);
        assert_eq!(s[0], 0.0);
        assert_eq!(s[1], 0.0);
    }

    #[test]
    fn apply_volume_full_volume_unchanged() {
        let mut s = vec![0.7f32, -0.3];
        apply_volume(&mut s, 1.0, false);
        assert!((s[0] - 0.7).abs() < 1e-6);
        assert!((s[1] + 0.3).abs() < 1e-6);
    }
}
