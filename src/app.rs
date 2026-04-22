use crate::stations::Station;
use std::sync::{atomic::AtomicBool, Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum StreamStatus {
    Connecting,
    Playing,
    Paused,
    Reconnecting,
    Error(String),
}

#[derive(Debug)]
pub enum ControlMsg {
    SwitchStation(usize),
    Quit,
}

pub struct AppState {
    pub stations: Vec<Station>,
    pub active_station_idx: usize,
    pub selected_station_idx: usize,
    pub show_station_panel: bool,
    pub is_paused: Arc<AtomicBool>,
    pub is_muted: Arc<AtomicBool>,
    pub volume: Arc<Mutex<f32>>,
    pub status: Arc<Mutex<StreamStatus>>,
    pub viz_buf: Arc<Mutex<Vec<f32>>>,
}

impl AppState {
    pub fn new(stations: Vec<Station>, station_idx: usize, volume_pct: u8) -> Self {
        Self {
            stations,
            active_station_idx: station_idx,
            selected_station_idx: station_idx,
            show_station_panel: false,
            is_paused: Arc::new(AtomicBool::new(false)),
            is_muted: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(Mutex::new(volume_pct as f32 / 100.0)),
            status: Arc::new(Mutex::new(StreamStatus::Connecting)),
            viz_buf: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn set_volume_pct(&self, pct: u8) {
        *self.volume.lock().unwrap() = pct.clamp(0, 100) as f32 / 100.0;
    }

    pub fn volume_pct(&self) -> u8 {
        (*self.volume.lock().unwrap() * 100.0).round() as u8
    }

    pub fn shared_clone(&self) -> Self {
        Self {
            stations: self.stations.clone(),
            active_station_idx: self.active_station_idx,
            selected_station_idx: self.selected_station_idx,
            show_station_panel: self.show_station_panel,
            is_paused: Arc::clone(&self.is_paused),
            is_muted: Arc::clone(&self.is_muted),
            volume: Arc::clone(&self.volume),
            status: Arc::clone(&self.status),
            viz_buf: Arc::clone(&self.viz_buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stations::Station;
    use std::sync::atomic::Ordering;

    fn sample_stations() -> Vec<Station> {
        vec![
            Station {
                name: "A".into(),
                slug: "a".into(),
                video_id: "1".into(),
            },
            Station {
                name: "B".into(),
                slug: "b".into(),
                video_id: "2".into(),
            },
        ]
    }

    #[test]
    fn new_has_correct_defaults() {
        let s = AppState::new(sample_stations(), 1, 75);
        assert_eq!(s.active_station_idx, 1);
        assert_eq!(s.selected_station_idx, 1);
        assert!(!s.show_station_panel);
        assert!(!s.is_paused.load(Ordering::Relaxed));
        assert!(!s.is_muted.load(Ordering::Relaxed));
        assert_eq!(s.volume_pct(), 75);
    }

    #[test]
    fn volume_pct_roundtrips() {
        let s = AppState::new(sample_stations(), 0, 60);
        s.set_volume_pct(45);
        assert_eq!(s.volume_pct(), 45);
    }

    #[test]
    fn volume_clamps_to_100() {
        let s = AppState::new(sample_stations(), 0, 80);
        s.set_volume_pct(150);
        assert_eq!(s.volume_pct(), 100);
    }

    #[test]
    fn shared_clone_shares_arcs() {
        let s = AppState::new(sample_stations(), 0, 80);
        let clone = s.shared_clone();
        s.is_paused.store(true, Ordering::Relaxed);
        assert!(clone.is_paused.load(Ordering::Relaxed));
    }

    #[test]
    fn stations_accessible() {
        let s = AppState::new(sample_stations(), 0, 80);
        assert_eq!(s.stations.len(), 2);
        assert_eq!(s.stations[1].slug, "b");
    }
}
