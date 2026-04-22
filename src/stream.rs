use crate::app::{AppState, ControlMsg, StreamStatus};
use crate::hls::{new_segments_since, resolve_media_playlist_url};
use crate::piped::resolve_hls_url;
use crate::ts::extract_aac_from_ts;
use anyhow::Result;
use crossbeam_channel::Receiver;
use std::collections::{HashSet, VecDeque};
use std::io::Cursor;
use std::sync::{Arc, Mutex, atomic::Ordering};
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const VIZ_BUF_MAX: usize = 8192;
/// Fixed output format. Every decoded segment is resampled and channel-
/// converted to this before being pushed into `audio_buf`, so the CPAL
/// callback can consume interleaved f32 samples at a known rate.
pub const OUTPUT_SAMPLE_RATE: u32 = 48_000;
pub const OUTPUT_CHANNELS: usize = 2;
// ~10 seconds at 48 kHz stereo. Enough to absorb a decoder spike without
// forcing the producer to block often, but small enough that audio stays
// close to the live edge.
const AUDIO_BUF_MAX: usize = OUTPUT_SAMPLE_RATE as usize * OUTPUT_CHANNELS * 10;
// Number of most-recent segments we actually decode on first playlist
// poll. The playlist typically lists ~6 segments of historical backlog;
// decoding all of them would add ~30 s of latency for no benefit.
const INITIAL_LIVE_SEGMENTS: usize = 2;
// Chunk size for producer-side pushes into audio_buf. Small chunks keep
// the mutex hold time well under one CPAL callback interval so the
// real-time audio thread never waits on the stream thread.
const PUSH_CHUNK: usize = 4096;

pub fn spawn_stream_thread(
    state: Arc<AppState>,
    audio_buf: Arc<Mutex<VecDeque<f32>>>,
    ctrl_rx: Receiver<ControlMsg>,
) {
    std::thread::spawn(move || {
        let mut station_idx = state.active_station_idx;
        loop {
            if let Ok(msg) = ctrl_rx.try_recv() {
                match msg {
                    ControlMsg::SwitchStation(idx) => station_idx = idx,
                    ControlMsg::Quit => return,
                }
            }
            let video_id = state.stations[station_idx].video_id.clone();
            *state.status.lock().unwrap() = StreamStatus::Connecting;
            match stream_station(&video_id, &state, &audio_buf, &ctrl_rx, &mut station_idx) {
                Ok(()) => {}
                Err(e) => {
                    log::error!("stream error: {}", e);
                    *state.status.lock().unwrap() = StreamStatus::Reconnecting;
                    std::thread::sleep(Duration::from_secs(3));
                }
            }
        }
    });
}

fn stream_station(
    video_id: &str,
    state: &Arc<AppState>,
    audio_buf: &Arc<Mutex<VecDeque<f32>>>,
    ctrl_rx: &Receiver<ControlMsg>,
    station_idx: &mut usize,
) -> Result<()> {
    let hls_manifest_url = resolve_hls_url(video_id)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let media_url = resolve_media_playlist_url(&hls_manifest_url, &client)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut first_poll = true;

    loop {
        if let Ok(msg) = ctrl_rx.try_recv() {
            return handle_ctrl(msg, station_idx);
        }
        if state.is_paused.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }

        let playlist_text = client.get(&media_url).send()?.text()?;
        let (mut new_segs, _ended) = new_segments_since(&media_url, &playlist_text, &seen);
        if first_poll {
            new_segs = take_live_tail(new_segs, INITIAL_LIVE_SEGMENTS, &mut seen);
            first_poll = false;
        }

        for seg_url in new_segs {
            if let Ok(msg) = ctrl_rx.try_recv() {
                return handle_ctrl(msg, station_idx);
            }
            while state.is_paused.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
            }

            let data = client.get(&seg_url).send()?.bytes()?.to_vec();
            let samples = decode_segment(&data)?;
            seen.insert(seg_url);
            push_samples(samples, audio_buf, &state.viz_buf);
            *state.status.lock().unwrap() = StreamStatus::Playing;
        }

        std::thread::sleep(Duration::from_secs(4));
    }
}

fn handle_ctrl(msg: ControlMsg, station_idx: &mut usize) -> Result<()> {
    if let ControlMsg::SwitchStation(idx) = msg {
        *station_idx = idx;
    }
    Ok(())
}

fn push_samples(
    samples: Vec<f32>,
    audio_buf: &Arc<Mutex<VecDeque<f32>>>,
    viz_buf: &Arc<Mutex<Vec<f32>>>,
) {
    // Stream samples into audio_buf in small chunks so the CPAL callback
    // (real-time thread) never waits long for the mutex. If the buffer
    // is full we sleep briefly and retry — crucially, without ever
    // evicting samples that are already queued for playback.
    let mut pos = 0;
    while pos < samples.len() {
        let wrote = {
            let mut buf = audio_buf.lock().unwrap();
            push_bounded(&samples[pos..], &mut buf, AUDIO_BUF_MAX, PUSH_CHUNK)
        };
        pos += wrote;
        if wrote == 0 {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    let mut viz = viz_buf.lock().unwrap();
    viz.extend_from_slice(&samples);
    if viz.len() > VIZ_BUF_MAX {
        let excess = viz.len() - VIZ_BUF_MAX;
        viz.drain(..excess);
    }
}

/// Push up to `chunk` samples from `src` into `buf`, never exceeding `cap`.
/// Returns how many were written. This function never evicts samples
/// already in `buf` — when the buffer is full it simply writes nothing
/// and returns 0, letting the caller throttle.
fn push_bounded(src: &[f32], buf: &mut VecDeque<f32>, cap: usize, chunk: usize) -> usize {
    let room = cap.saturating_sub(buf.len()).min(chunk);
    let n = room.min(src.len());
    buf.extend(src[..n].iter().copied());
    n
}

/// Keep only the final `keep` segments of `segs` (closest to live edge)
/// and insert the rest into `seen` so we never retroactively decode them.
fn take_live_tail(segs: Vec<String>, keep: usize, seen: &mut HashSet<String>) -> Vec<String> {
    if segs.len() <= keep {
        return segs;
    }
    let skip = segs.len() - keep;
    let mut out = Vec::with_capacity(keep);
    for (i, s) in segs.into_iter().enumerate() {
        if i < skip {
            seen.insert(s);
        } else {
            out.push(s);
        }
    }
    out
}

pub fn decode_segment(data: &[u8]) -> Result<Vec<f32>> {
    // Piped's HLS segments are MPEG-TS — symphonia can't demux that,
    // so strip the TS framing and feed the raw ADTS AAC payload.
    let aac = extract_aac_from_ts(data)?;
    let cursor = Cursor::new(aac);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("aac");
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow::anyhow!("no audio track in segment"))?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())?;

    let mut all_samples: Vec<f32> = Vec::new();
    let mut source_spec: Option<(u32, usize)> = None;
    loop {
        match format.next_packet() {
            Ok(packet) => {
                let decoded = match decoder.decode(&packet) {
                    Ok(d) => d,
                    // Skip malformed packets rather than tearing down the
                    // whole segment (and reconnecting) on a single
                    // corrupt frame.
                    Err(symphonia::core::errors::Error::DecodeError(msg)) => {
                        log::debug!("skipping bad AAC packet: {}", msg);
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                };
                let spec = *decoded.spec();
                source_spec.get_or_insert_with(|| {
                    log::debug!(
                        "decoded segment spec: rate={} Hz, channels={}",
                        spec.rate,
                        spec.channels.count()
                    );
                    (spec.rate, spec.channels.count())
                });
                let mut sample_buf =
                    SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                sample_buf.copy_interleaved_ref(decoded);
                all_samples.extend_from_slice(sample_buf.samples());
            }
            Err(symphonia::core::errors::Error::IoError(_)) => break,
            Err(e) => return Err(e.into()),
        }
    }

    let (in_rate, in_channels) = source_spec.ok_or_else(|| anyhow::anyhow!("no audio frames decoded"))?;
    Ok(resample_interleaved(
        &all_samples,
        in_rate,
        in_channels,
        OUTPUT_SAMPLE_RATE,
        OUTPUT_CHANNELS,
    ))
}

/// Convert interleaved f32 samples from `(in_rate, in_ch)` to
/// `(out_rate, out_ch)` using linear interpolation. This is a simple
/// resampler — good enough for speech/music at common ratios and
/// dramatically cheaper than a polyphase filter. Pitch and tempo are
/// preserved; high-frequency aliasing can occur when downsampling but
/// we only ever upsample here.
pub fn resample_interleaved(
    samples: &[f32],
    in_rate: u32,
    in_ch: usize,
    out_rate: u32,
    out_ch: usize,
) -> Vec<f32> {
    if samples.is_empty() || in_ch == 0 || out_ch == 0 {
        return Vec::new();
    }
    let converted = convert_channels(samples, in_ch, out_ch);
    if in_rate == out_rate {
        return converted;
    }
    let in_frames = converted.len() / out_ch;
    if in_frames == 0 {
        return Vec::new();
    }
    let out_frames = (in_frames as u64 * out_rate as u64 / in_rate as u64) as usize;
    let mut out = Vec::with_capacity(out_frames * out_ch);
    let ratio = in_rate as f64 / out_rate as f64;
    for i in 0..out_frames {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos as usize;
        let frac = (src_pos - src_idx as f64) as f32;
        let next_idx = (src_idx + 1).min(in_frames - 1);
        for c in 0..out_ch {
            let a = converted[src_idx * out_ch + c];
            let b = converted[next_idx * out_ch + c];
            out.push(a + (b - a) * frac);
        }
    }
    out
}

fn convert_channels(samples: &[f32], in_ch: usize, out_ch: usize) -> Vec<f32> {
    if in_ch == out_ch {
        return samples.to_vec();
    }
    match (in_ch, out_ch) {
        (1, 2) => samples.iter().flat_map(|&s| [s, s]).collect(),
        (2, 1) => samples.chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect(),
        _ => {
            // Fallback: take first `out_ch` channels of each input frame,
            // or duplicate the last if fewer are available. Not exactly
            // correct for multi-channel surround but won't crash.
            let in_frames = samples.len() / in_ch;
            let mut out = Vec::with_capacity(in_frames * out_ch);
            for f in 0..in_frames {
                for c in 0..out_ch {
                    let src_c = c.min(in_ch - 1);
                    out.push(samples[f * in_ch + src_c]);
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_bounded_fills_available_room() {
        let mut buf: VecDeque<f32> = VecDeque::new();
        let wrote = push_bounded(&[1.0, 2.0, 3.0, 4.0], &mut buf, 10, 8);
        assert_eq!(wrote, 4);
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.front(), Some(&1.0));
    }

    #[test]
    fn push_bounded_caps_at_chunk_size() {
        let mut buf: VecDeque<f32> = VecDeque::new();
        let wrote = push_bounded(&[1.0; 10], &mut buf, 100, 4);
        assert_eq!(wrote, 4);
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn push_bounded_caps_at_remaining_capacity() {
        let mut buf: VecDeque<f32> = VecDeque::from(vec![0.0; 8]);
        let wrote = push_bounded(&[1.0; 10], &mut buf, 10, 100);
        assert_eq!(wrote, 2);
        assert_eq!(buf.len(), 10);
    }

    // Regression: the previous implementation evicted the oldest sample
    // on every push when full. Those samples are the ones about to be
    // played, so eviction produced audible skips. `push_bounded` must
    // never touch the existing queued samples.
    #[test]
    fn push_bounded_never_evicts_existing_samples() {
        let existing = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let mut buf: VecDeque<f32> = VecDeque::from(existing.clone());
        let wrote = push_bounded(&[9.9; 100], &mut buf, 5, 32);
        assert_eq!(wrote, 0);
        let kept: Vec<f32> = buf.iter().copied().collect();
        assert_eq!(kept, existing);
    }

    #[test]
    fn take_live_tail_keeps_last_n_and_marks_rest_seen() {
        let segs: Vec<String> = (0..6).map(|i| format!("https://h/s{}.ts", i)).collect();
        let mut seen = HashSet::new();
        let kept = take_live_tail(segs.clone(), 2, &mut seen);
        assert_eq!(kept, vec!["https://h/s4.ts".to_string(), "https://h/s5.ts".to_string()]);
        // The four skipped segments must be remembered as seen so a
        // follow-up poll does not replay them.
        for i in 0..4 {
            assert!(seen.contains(&format!("https://h/s{}.ts", i)), "s{} not marked seen", i);
        }
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn take_live_tail_noop_when_fewer_than_keep() {
        let segs = vec!["a".to_string(), "b".to_string()];
        let mut seen = HashSet::new();
        let kept = take_live_tail(segs.clone(), 5, &mut seen);
        assert_eq!(kept, segs);
        assert!(seen.is_empty());
    }

    // Regression: HE-AAC core output is 22050 Hz; CPAL runs at 48000 Hz.
    // Without resampling, samples drained twice as fast as supplied, so
    // audio stuttered. The resampler must produce the expected number
    // of output frames for a given input count.
    #[test]
    fn resample_22050_to_48000_matches_frame_count() {
        // 1 second of stereo audio at 22050 Hz.
        let input: Vec<f32> = (0..22050).flat_map(|_| [0.5, -0.5]).collect();
        let out = resample_interleaved(&input, 22050, 2, 48000, 2);
        // 48000 / 22050 * 22050 ≈ 48000 output frames (±1 for rounding).
        let out_frames = out.len() / 2;
        assert!(
            (47999..=48001).contains(&out_frames),
            "expected ~48000 stereo frames, got {}",
            out_frames
        );
    }

    #[test]
    fn resample_is_identity_when_rates_and_channels_match() {
        let input: Vec<f32> = vec![0.1, -0.2, 0.3, -0.4];
        let out = resample_interleaved(&input, 48000, 2, 48000, 2);
        assert_eq!(out, input);
    }

    #[test]
    fn resample_mono_to_stereo_duplicates_channel() {
        let input = vec![0.1, 0.2, 0.3];
        let out = resample_interleaved(&input, 48000, 1, 48000, 2);
        assert_eq!(out, vec![0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
    }

    #[test]
    fn resample_stereo_to_mono_averages_channels() {
        let input = vec![0.2, 0.4, -0.2, 0.0];
        let out = resample_interleaved(&input, 48000, 2, 48000, 1);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.3).abs() < 1e-6);
        assert!((out[1] + 0.1).abs() < 1e-6);
    }

    // A constant DC input must stay constant through resampling —
    // interpolating between equal values yields the same value.
    #[test]
    fn resample_preserves_constant_signal() {
        let input = vec![0.5; 44100 * 2];
        let out = resample_interleaved(&input, 44100, 2, 48000, 2);
        assert!(!out.is_empty());
        for s in &out {
            assert!((s - 0.5).abs() < 1e-5, "sample drifted to {}", s);
        }
    }

    #[test]
    fn resample_empty_input_produces_empty_output() {
        let out = resample_interleaved(&[], 22050, 2, 48000, 2);
        assert!(out.is_empty());
    }

    #[test]
    #[ignore = "diagnostic: reads a real Piped segment from /tmp/seg.ts"]
    fn decode_real_segment_yields_output_rate_samples() {
        let data = std::fs::read("/tmp/seg.ts").expect("/tmp/seg.ts must exist");
        let samples = decode_segment(&data).expect("decode ok");
        let secs = samples.len() as f32 / (OUTPUT_SAMPLE_RATE as f32 * OUTPUT_CHANNELS as f32);
        eprintln!(
            "decode_segment produced {} samples ≈ {:.2} s at {} Hz × {} ch",
            samples.len(),
            secs,
            OUTPUT_SAMPLE_RATE,
            OUTPUT_CHANNELS
        );
        // A Lofi Girl HLS segment is ~5 seconds. After resampling from
        // 22050 Hz stereo to 48000 Hz stereo, expect ~480,000 samples.
        assert!(
            samples.len() > 48_000 * OUTPUT_CHANNELS * 4,
            "too few samples — expected at least 4 s of audio, got {:.2} s",
            secs
        );
        assert!(
            samples.len() < 48_000 * OUTPUT_CHANNELS * 7,
            "too many samples — expected at most 7 s of audio, got {:.2} s",
            secs
        );
    }
}
