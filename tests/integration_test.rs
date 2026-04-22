use locli_girl::app::AppState;
use locli_girl::config::Config;
use locli_girl::stations::{
    extract_livestreams_tab_data, find_by_slug, parse_livestreams_tab, Station,
};

fn sample_stations() -> Vec<Station> {
    vec![
        Station { name: "A".into(), slug: "a".into(), video_id: "v1".into() },
        Station { name: "B".into(), slug: "b".into(), video_id: "v2".into() },
    ]
}

#[test]
fn default_config_volume_is_80() {
    assert_eq!(Config::default().volume, 80);
}

#[test]
fn default_config_slug_is_non_empty() {
    assert!(!Config::default().last_station_slug.is_empty());
}

#[test]
fn app_state_initializes_for_every_index() {
    let stations = sample_stations();
    for i in 0..stations.len() {
        let s = AppState::new(stations.clone(), i, 80);
        assert_eq!(s.active_station_idx, i);
        assert_eq!(s.volume_pct(), 80);
    }
}

#[test]
fn find_by_slug_locates_station() {
    let stations = sample_stations();
    assert_eq!(find_by_slug(&stations, "b").unwrap().video_id, "v2");
    assert!(find_by_slug(&stations, "missing").is_none());
}

// Regression: end-to-end shape of the current Piped API — a channel
// response carries only a `tabs` array pointing at the livestreams
// subtab; that subtab's response is what actually lists live stations.
#[test]
fn livestreams_tab_flow_integration() {
    let channel_json = r#"{
        "relatedStreams": [],
        "tabs": [
            {"name": "shorts", "data": "shorts-blob"},
            {"name": "livestreams", "data": "live-blob"}
        ]
    }"#;
    let tab_data = extract_livestreams_tab_data(channel_json).unwrap();
    assert_eq!(tab_data, "live-blob");

    let tab_json = r#"{
        "nextpage": null,
        "content": [
            {"title": "lofi hip hop radio 📚 - beats to relax/study to", "url": "/watch?v=jfKfPfyJRdk", "duration": -1},
            {"title": "Past live",                                       "url": "/watch?v=oldvid",      "duration": 10800}
        ]
    }"#;
    let stations = parse_livestreams_tab(tab_json).unwrap();
    assert_eq!(stations.len(), 1);
    assert_eq!(stations[0].video_id, "jfKfPfyJRdk");
    assert_eq!(stations[0].slug, "lofi-hip-hop-radio-beats-to-relax-study-to");
}
