use crate::app::{AppState, StreamStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::sync::atomic::Ordering;

pub fn render_immersive(frame: &mut Frame, area: Rect, state: &AppState, bars: &[f32]) {
    render_viz(frame, area, bars);
    render_overlay(frame, area, state);
}

pub fn render_sidebar(frame: &mut Frame, area: Rect, state: &AppState, bars: &[f32]) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(0)])
        .split(area);

    render_station_list(frame, chunks[0], state);

    let right = chunks[1];
    let station = &state.stations[state.active_station_idx];
    let status_text = status_string(state);
    let header = Paragraph::new(Line::from(vec![
        Span::raw(station.name.as_str()),
        Span::raw("  "),
        Span::styled(status_text, Style::default().add_modifier(Modifier::BOLD)),
    ]));
    let header_area = Rect {
        x: right.x,
        y: right.y,
        width: right.width,
        height: 1,
    };
    frame.render_widget(header, header_area);

    let viz_area = Rect {
        x: right.x,
        y: right.y + 1,
        width: right.width,
        height: right.height.saturating_sub(4),
    };
    render_viz(frame, viz_area, bars);
    render_vol_hints(frame, right, state);
}

fn render_viz(frame: &mut Frame, area: Rect, bars: &[f32]) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let n_bars = area.width as usize;
    let max_rows = area.height.saturating_sub(1) as usize;
    if max_rows == 0 {
        return;
    }

    let resampled = resample_bars(bars, n_bars);
    for (col, &height) in resampled.iter().enumerate() {
        let filled = (height * max_rows as f32).round() as u16;
        for row in 0..filled {
            let x = area.x + col as u16;
            let y = area.y + area.height - 1 - row;
            if x < area.x + area.width && y >= area.y {
                if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                    cell.set_symbol("█");
                }
            }
        }
    }
}

fn resample_bars(bars: &[f32], n: usize) -> Vec<f32> {
    if bars.is_empty() {
        return vec![0.0; n];
    }
    (0..n)
        .map(|i| {
            let idx = (i as f32 * bars.len() as f32 / n as f32) as usize;
            bars.get(idx).copied().unwrap_or(0.0)
        })
        .collect()
}

fn render_overlay(frame: &mut Frame, area: Rect, state: &AppState) {
    if area.height < 4 {
        return;
    }
    let station = &state.stations[state.active_station_idx];
    let name_area = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(14),
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(format!("locli-girl  {}", station.name)),
        name_area,
    );

    let status = status_string(state);
    let sw = status.len().min(14) as u16;
    let status_area = Rect {
        x: area.x + area.width.saturating_sub(sw + 1),
        y: area.y + 1,
        width: sw,
        height: 1,
    };
    frame.render_widget(Paragraph::new(status), status_area);
    render_vol_hints(frame, area, state);
}

fn render_vol_hints(frame: &mut Frame, area: Rect, state: &AppState) {
    if area.height < 3 {
        return;
    }
    let vol_pct = state.volume_pct();
    let muted = state.is_muted.load(Ordering::Relaxed);
    let filled = if muted {
        0
    } else {
        (vol_pct as usize * 20 / 100).min(20)
    };
    let vol_str = if muted {
        "VOL [muted]              ".to_string()
    } else {
        format!(
            "VOL [{}{}] {}%",
            "█".repeat(filled),
            "░".repeat(20 - filled),
            vol_pct
        )
    };
    let vol_area = Rect {
        x: area.x + 1,
        y: area.y + area.height - 3,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(Paragraph::new(vol_str), vol_area);

    let hints = "space · +/- · m · q                           s stations";
    let hints_area = Rect {
        x: area.x + 1,
        y: area.y + area.height - 2,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(Paragraph::new(hints), hints_area);
}

fn render_station_list(frame: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .stations
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let prefix = if i == state.active_station_idx {
                "▶ "
            } else {
                "  "
            };
            ListItem::new(format!("{}{}", prefix, s.name))
        })
        .collect();
    let mut list_state = ListState::default();
    list_state.select(Some(state.selected_station_idx));
    let list = List::new(items)
        .block(Block::default().borders(Borders::RIGHT).title("STATIONS"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn status_string(state: &AppState) -> String {
    match &*state.status.lock().unwrap() {
        StreamStatus::Playing | StreamStatus::Paused => "● LIVE".to_string(),
        StreamStatus::Connecting => "connecting…".to_string(),
        StreamStatus::Reconnecting => "reconnecting…".to_string(),
        StreamStatus::Error(e) => format!("error: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppState;
    use crate::stations::Station;
    use ratatui::{backend::TestBackend, Terminal};

    fn sample_stations() -> Vec<Station> {
        vec![
            Station {
                name: "Station A".into(),
                slug: "a".into(),
                video_id: "1".into(),
            },
            Station {
                name: "Station B".into(),
                slug: "b".into(),
                video_id: "2".into(),
            },
        ]
    }

    fn terminal(w: u16, h: u16) -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(w, h)).unwrap()
    }

    #[test]
    fn render_immersive_does_not_panic() {
        let mut t = terminal(80, 24);
        let state = AppState::new(sample_stations(), 0, 75);
        t.draw(|f| render_immersive(f, f.area(), &state, &[]))
            .unwrap();
    }

    #[test]
    fn render_sidebar_does_not_panic() {
        let mut t = terminal(80, 24);
        let state = AppState::new(sample_stations(), 0, 75);
        t.draw(|f| render_sidebar(f, f.area(), &state, &[]))
            .unwrap();
    }

    #[test]
    fn render_tiny_terminal_does_not_panic() {
        let mut t = terminal(10, 3);
        let state = AppState::new(sample_stations(), 0, 75);
        t.draw(|f| render_immersive(f, f.area(), &state, &[]))
            .unwrap();
    }

    #[test]
    fn render_with_bars_does_not_panic() {
        let mut t = terminal(80, 24);
        let state = AppState::new(sample_stations(), 0, 75);
        let bars: Vec<f32> = (0..32).map(|i| i as f32 / 32.0).collect();
        t.draw(|f| render_immersive(f, f.area(), &state, &bars))
            .unwrap();
    }
}
