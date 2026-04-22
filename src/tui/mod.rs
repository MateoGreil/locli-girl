pub mod layout;

use crate::app::{AppState, ControlMsg};
use crate::visualizer::compute_bars;
use anyhow::Result;
use crossbeam_channel::Sender;
use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::stdout;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub fn run_tui(state: &mut AppState, ctrl_tx: Sender<ControlMsg>) -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, state, &ctrl_tx);

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    state: &mut AppState,
    ctrl_tx: &Sender<ControlMsg>,
) -> Result<()> {
    let frame_duration = Duration::from_millis(1000 / 60);
    loop {
        terminal.draw(|f| {
            let area = f.area();
            let viz = state.viz_buf.lock().unwrap().clone();
            let bars = compute_bars(&viz, area.width as usize, 44100.0);
            if state.show_station_panel {
                layout::render_sidebar(f, area, state, &bars);
            } else {
                layout::render_immersive(f, area, state, &bars);
            }
        })?;

        if event::poll(frame_duration)? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    ctrl_tx.send(ControlMsg::Quit).ok();
                    return Ok(());
                }
                handle_key(key.code, state, ctrl_tx);
            }
        }
    }
}

pub fn handle_key(code: KeyCode, state: &mut AppState, ctrl_tx: &Sender<ControlMsg>) {
    match code {
        KeyCode::Char(' ') => {
            let p = state.is_paused.load(Ordering::Relaxed);
            state.is_paused.store(!p, Ordering::Relaxed);
        }
        KeyCode::Char('m') => {
            let m = state.is_muted.load(Ordering::Relaxed);
            state.is_muted.store(!m, Ordering::Relaxed);
        }
        KeyCode::Char('s') => {
            state.show_station_panel = !state.show_station_panel;
        }
        KeyCode::Esc => {
            state.show_station_panel = false;
        }
        KeyCode::Char('+') => {
            state.set_volume_pct(state.volume_pct().saturating_add(5).min(100));
        }
        KeyCode::Char('-') => {
            state.set_volume_pct(state.volume_pct().saturating_sub(5));
        }
        KeyCode::Down if state.show_station_panel => {
            state.selected_station_idx =
                (state.selected_station_idx + 1).min(state.stations.len() - 1);
        }
        KeyCode::Up if state.show_station_panel => {
            state.selected_station_idx = state.selected_station_idx.saturating_sub(1);
        }
        KeyCode::Enter if state.show_station_panel => {
            let idx = state.selected_station_idx;
            state.active_station_idx = idx;
            state.show_station_panel = false;
            ctrl_tx.send(ControlMsg::SwitchStation(idx)).ok();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppState;
    use crate::stations::Station;
    use crossbeam_channel::unbounded;
    use crossterm::event::KeyCode;

    fn stations() -> Vec<Station> {
        (0..5)
            .map(|i| Station {
                name: format!("Station {i}"),
                slug: format!("station-{i}"),
                video_id: format!("vid{i}"),
            })
            .collect()
    }

    fn st() -> AppState {
        AppState::new(stations(), 0, 80)
    }

    #[test]
    fn space_toggles_pause() {
        let (tx, _rx) = unbounded();
        let mut s = st();
        handle_key(KeyCode::Char(' '), &mut s, &tx);
        assert!(s.is_paused.load(std::sync::atomic::Ordering::Relaxed));
        handle_key(KeyCode::Char(' '), &mut s, &tx);
        assert!(!s.is_paused.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn m_toggles_mute() {
        let (tx, _rx) = unbounded();
        let mut s = st();
        handle_key(KeyCode::Char('m'), &mut s, &tx);
        assert!(s.is_muted.load(std::sync::atomic::Ordering::Relaxed));
        handle_key(KeyCode::Char('m'), &mut s, &tx);
        assert!(!s.is_muted.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn s_toggles_station_panel() {
        let (tx, _rx) = unbounded();
        let mut s = st();
        handle_key(KeyCode::Char('s'), &mut s, &tx);
        assert!(s.show_station_panel);
        handle_key(KeyCode::Char('s'), &mut s, &tx);
        assert!(!s.show_station_panel);
    }

    #[test]
    fn esc_closes_station_panel() {
        let (tx, _rx) = unbounded();
        let mut s = st();
        s.show_station_panel = true;
        handle_key(KeyCode::Esc, &mut s, &tx);
        assert!(!s.show_station_panel);
    }

    #[test]
    fn plus_increases_volume_by_5() {
        let (tx, _rx) = unbounded();
        let mut s = AppState::new(stations(), 0, 70);
        handle_key(KeyCode::Char('+'), &mut s, &tx);
        assert_eq!(s.volume_pct(), 75);
    }

    #[test]
    fn minus_decreases_volume_by_5() {
        let (tx, _rx) = unbounded();
        let mut s = AppState::new(stations(), 0, 70);
        handle_key(KeyCode::Char('-'), &mut s, &tx);
        assert_eq!(s.volume_pct(), 65);
    }

    #[test]
    fn volume_does_not_exceed_100() {
        let (tx, _rx) = unbounded();
        let mut s = AppState::new(stations(), 0, 98);
        handle_key(KeyCode::Char('+'), &mut s, &tx);
        assert_eq!(s.volume_pct(), 100);
    }

    #[test]
    fn down_moves_selection_when_panel_open() {
        let (tx, _rx) = unbounded();
        let mut s = st();
        s.show_station_panel = true;
        handle_key(KeyCode::Down, &mut s, &tx);
        assert_eq!(s.selected_station_idx, 1);
    }

    #[test]
    fn enter_sends_switch_and_closes_panel() {
        let (tx, rx) = unbounded();
        let mut s = st();
        s.show_station_panel = true;
        s.selected_station_idx = 3;
        handle_key(KeyCode::Enter, &mut s, &tx);
        assert!(!s.show_station_panel);
        assert_eq!(s.active_station_idx, 3);
        match rx.recv().unwrap() {
            crate::app::ControlMsg::SwitchStation(i) => assert_eq!(i, 3),
            _ => panic!("wrong msg"),
        }
    }
}
