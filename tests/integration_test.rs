use locli_girl::app::AppState;
use locli_girl::config::Config;
use locli_girl::stations::{find_by_slug, parse_channel_response, Station};

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

#[test]
fn parse_channel_response_integration() {
    let json = r#"{
        "relatedStreams": [
            {"title": "Lofi Hip Hop Radio 📚 - beats to relax/study to", "url": "/watch?v=abc123", "isLive": true},
            {"title": "Not live video", "url": "/watch?v=xyz", "isLive": false}
        ]
    }"#;
    let stations = parse_channel_response(json).unwrap();
    assert_eq!(stations.len(), 1);
    assert_eq!(stations[0].video_id, "abc123");
    assert!(!stations[0].slug.is_empty());
}
