use anyhow::{anyhow, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Station {
    pub name: String,
    pub slug: String,
    pub video_id: String,
}

#[derive(Deserialize)]
struct ChannelResponse {
    #[serde(default)]
    tabs: Vec<ChannelTab>,
}

#[derive(Deserialize)]
struct ChannelTab {
    name: String,
    data: String,
}

#[derive(Deserialize)]
struct TabResponse {
    #[serde(default)]
    content: Vec<StreamEntry>,
}

#[derive(Deserialize)]
struct StreamEntry {
    title: String,
    url: String,
    // Piped marks currently-live streams with duration == -1. The older
    // `isLive` field was dropped from recent API versions, so duration is
    // the reliable marker.
    #[serde(default)]
    duration: i64,
}

/// Extract the opaque `data` blob for the channel's `livestreams` subtab.
/// This string must be passed to `/channels/tabs?data=...` to list live
/// streams — the main `/channel/{id}` response no longer contains them.
pub fn extract_livestreams_tab_data(json: &str) -> Result<String> {
    let channel: ChannelResponse = serde_json::from_str(json)?;
    channel
        .tabs
        .into_iter()
        .find(|t| t.name == "livestreams")
        .map(|t| t.data)
        .ok_or_else(|| anyhow!("channel has no 'livestreams' tab"))
}

/// Parse the response of `/channels/tabs?data=...` (the livestreams tab)
/// and return currently-live stations (duration == -1).
pub fn parse_livestreams_tab(json: &str) -> Result<Vec<Station>> {
    let tab: TabResponse = serde_json::from_str(json)?;
    Ok(tab
        .content
        .into_iter()
        .filter(is_live)
        .filter_map(to_station)
        .collect())
}

pub fn find_by_slug<'a>(stations: &'a [Station], slug: &str) -> Option<&'a Station> {
    stations.iter().find(|s| s.slug == slug)
}

pub fn slugify(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let result = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    if result.is_empty() {
        "unknown".to_string()
    } else {
        result
    }
}

fn is_live(s: &StreamEntry) -> bool {
    s.duration == -1
}

fn to_station(s: StreamEntry) -> Option<Station> {
    extract_video_id(&s.url).map(|vid| Station {
        slug: slugify(&s.title),
        name: s.title,
        video_id: vid,
    })
}

fn extract_video_id(url: &str) -> Option<String> {
    url.split("v=")
        .nth(1)
        .map(|s| s.split('&').next().unwrap_or(s).to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_hyphenates() {
        assert_eq!(slugify("Lofi Hip Hop Radio"), "lofi-hip-hop-radio");
    }

    #[test]
    fn slugify_strips_emojis_and_special_chars() {
        assert_eq!(
            slugify("lofi hip hop radio 📚 - beats"),
            "lofi-hip-hop-radio-beats"
        );
    }

    #[test]
    fn extract_video_id_from_watch_url() {
        assert_eq!(
            extract_video_id("/watch?v=jfKfPfyJRdk"),
            Some("jfKfPfyJRdk".to_string())
        );
    }

    #[test]
    fn extract_video_id_with_extra_params() {
        assert_eq!(
            extract_video_id("/watch?v=abc123&t=10"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn find_by_slug_returns_correct_station() {
        let stations = vec![
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
        ];
        assert_eq!(find_by_slug(&stations, "b").unwrap().video_id, "2");
        assert!(find_by_slug(&stations, "c").is_none());
    }

    // Regression: Piped's current API stores live streams in the
    // livestreams *tab*, not in the channel's relatedStreams. We must
    // be able to pull the tab's `data` blob out of the channel response.
    #[test]
    fn extract_livestreams_tab_data_finds_livestreams_entry() {
        let json = r#"{
            "relatedStreams": [],
            "tabs": [
                {"name": "shorts", "data": "shorts-data-blob"},
                {"name": "livestreams", "data": "live-data-blob"},
                {"name": "albums", "data": "albums-data-blob"}
            ]
        }"#;
        assert_eq!(
            extract_livestreams_tab_data(json).unwrap(),
            "live-data-blob"
        );
    }

    #[test]
    fn extract_livestreams_tab_data_errors_when_tab_missing() {
        let json = r#"{"relatedStreams": [], "tabs": [{"name": "shorts", "data": "x"}]}"#;
        assert!(extract_livestreams_tab_data(json).is_err());
    }

    #[test]
    fn extract_livestreams_tab_data_errors_on_no_tabs_key() {
        // Older instances may omit `tabs` entirely; must error, not panic.
        let json = r#"{"relatedStreams": []}"#;
        assert!(extract_livestreams_tab_data(json).is_err());
    }

    // Regression: the livestreams tab response contains a `content` array
    // with entries whose `duration == -1` are currently live. Others
    // (past streams) have actual durations and must be filtered out.
    #[test]
    fn parse_livestreams_tab_filters_by_duration_marker() {
        let json = r#"{
            "nextpage": null,
            "content": [
                {"title": "Live Radio",   "url": "/watch?v=live1", "duration": -1},
                {"title": "Past Stream",  "url": "/watch?v=past1", "duration": 43200},
                {"title": "Other Live",   "url": "/watch?v=live2", "duration": -1}
            ]
        }"#;
        let stations = parse_livestreams_tab(json).unwrap();
        assert_eq!(stations.len(), 2);
        let ids: Vec<&str> = stations.iter().map(|s| s.video_id.as_str()).collect();
        assert!(ids.contains(&"live1"));
        assert!(ids.contains(&"live2"));
    }

    // Regression: a tab response with no `content` key (empty channel,
    // malformed-but-valid JSON) must decode to an empty Vec, not panic.
    #[test]
    fn parse_livestreams_tab_handles_missing_content() {
        let stations = parse_livestreams_tab(r#"{"nextpage": null}"#).unwrap();
        assert!(stations.is_empty());
    }

    // Regression: the `isLive` boolean was removed from recent Piped API
    // responses. An entry missing both `isLive` and `duration` must not
    // be treated as live (the old code defaulted isLive to false, which
    // happened to be correct, but defaulting duration to 0 is also
    // not-live — this pins that behavior).
    #[test]
    fn entry_without_duration_is_not_live() {
        let json = r#"{
            "content": [
                {"title": "No duration field", "url": "/watch?v=xx"}
            ]
        }"#;
        let stations = parse_livestreams_tab(json).unwrap();
        assert!(stations.is_empty());
    }
}
