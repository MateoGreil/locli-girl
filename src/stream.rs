use crate::app::{AppState, ControlMsg, StreamStatus};
use crate::hls::{parse_media_playlist, resolve_media_playlist_url};
use crate::piped::resolve_hls_url;
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
const AUDIO_BUF_MAX: usize = 44100 * 8;

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

    loop {
        if let Ok(msg) = ctrl_rx.try_recv() {
            return handle_ctrl(msg, station_idx);
        }
        if state.is_paused.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }

        let playlist_text = client.get(&media_url).send()?.text()?;
        let (seg_urls, _) = parse_media_playlist(&playlist_text);
        let new_segs: Vec<String> =
            seg_urls.into_iter().filter(|u| !seen.contains(u)).collect();

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
    {
        let mut buf = audio_buf.lock().unwrap();
        for s in &samples {
            if buf.len() >= AUDIO_BUF_MAX {
                buf.pop_front();
            }
            buf.push_back(*s);
        }
    }
    {
        let mut viz = viz_buf.lock().unwrap();
        viz.extend_from_slice(&samples);
        if viz.len() > VIZ_BUF_MAX {
            let excess = viz.len() - VIZ_BUF_MAX;
            viz.drain(..excess);
        }
    }
}

pub fn decode_segment(data: &[u8]) -> Result<Vec<f32>> {
    let cursor = Cursor::new(data.to_vec());
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
    loop {
        match format.next_packet() {
            Ok(packet) => {
                let decoded = decoder.decode(&packet)?;
                let spec = *decoded.spec();
                let mut sample_buf =
                    SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                sample_buf.copy_interleaved_ref(decoded);
                all_samples.extend_from_slice(sample_buf.samples());
            }
            Err(symphonia::core::errors::Error::IoError(_)) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(all_samples)
}
