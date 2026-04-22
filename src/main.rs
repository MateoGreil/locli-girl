mod app;
mod config;
mod hls;
mod mpris;
mod piped;
mod player;
mod stations;
mod stream;
mod ts;
mod tui;
mod visualizer;

use anyhow::Result;
use app::ControlMsg;
use config::Config;
use stations::find_by_slug;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

fn main() -> Result<()> {
    env_logger::init();

    let cfg = Config::load()?;

    let fetched = piped::fetch_stations().unwrap_or_else(|e| {
        log::error!("failed to fetch stations: {e}");
        std::process::exit(1);
    });

    let station_idx = find_by_slug(&fetched, &cfg.last_station_slug)
        .and_then(|s| fetched.iter().position(|st| st.slug == s.slug))
        .unwrap_or(0);

    let mut state = app::AppState::new(fetched, station_idx, cfg.volume);
    let audio_buf: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
    let (ctrl_tx, ctrl_rx) = crossbeam_channel::unbounded::<ControlMsg>();

    let _audio_stream = player::start_audio_output(
        Arc::clone(&audio_buf),
        Arc::clone(&state.is_paused),
        Arc::clone(&state.volume),
        Arc::clone(&state.is_muted),
    )?;

    stream::spawn_stream_thread(
        Arc::new(state.shared_clone()),
        Arc::clone(&audio_buf),
        ctrl_rx,
    );

    // MPRIS is best-effort — silent failure if D-Bus is unavailable
    let _mpris = mpris::start_mpris(Arc::new(state.shared_clone())).ok();

    tui::run_tui(&mut state, ctrl_tx)?;

    Config {
        last_station_slug: state.stations[state.active_station_idx].slug.clone(),
        volume: state.volume_pct(),
    }
    .save()
    .ok();

    Ok(())
}
