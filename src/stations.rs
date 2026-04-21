use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Station {
    pub name: String,
    pub slug: String,
    pub video_id: String,
}

#[derive(Deserialize)]
struct ChannelResponse {
    #[serde(rename = "relatedStreams")]
    related_streams: Vec<StreamEntry>,
}

#[derive(Deserialize)]
struct StreamEntry {
    title: String,
    url: String,
    #[serde(rename = "isLive", default)]
    is_live: bool,
}

pub fn parse_channel_response(json: &str) -> Result<Vec<Station>> {
    let channel: ChannelResponse = serde_json::from_str(json)?;
    let stations = channel
        .related_streams
        .into_iter()
        .filter(|s| s.is_live)
        .filter_map(|s| {
            extract_video_id(&s.url).map(|vid| Station {
                slug: slugify(&s.title),
                name: s.title,
                video_id: vid,
            })
        })
        .collect();
    Ok(stations)
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
    if result.is_empty() { "unknown".to_string() } else { result }
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
        assert_eq!(slugify("lofi hip hop radio 📚 - beats"), "lofi-hip-hop-radio-beats");
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
    fn parse_channel_response_filters_live() {
        let json = r#"{
            "relatedStreams": [
                {"title": "Live Stream", "url": "/watch?v=abc123", "isLive": true},
                {"title": "Not Live",    "url": "/watch?v=xyz789", "isLive": false}
            ]
        }"#;
        let stations = parse_channel_response(json).unwrap();
        assert_eq!(stations.len(), 1);
        assert_eq!(stations[0].video_id, "abc123");
    }

    #[test]
    fn parse_channel_response_generates_slug() {
        let json = r#"{
            "relatedStreams": [
                {"title": "lofi hip hop radio 📚 - beats to relax/study to", "url": "/watch?v=abc", "isLive": true}
            ]
        }"#;
        let stations = parse_channel_response(json).unwrap();
        assert_eq!(stations[0].slug, "lofi-hip-hop-radio-beats-to-relax-study-to");
    }

    #[test]
    fn find_by_slug_returns_correct_station() {
        let stations = vec![
            Station { name: "A".into(), slug: "a".into(), video_id: "1".into() },
            Station { name: "B".into(), slug: "b".into(), video_id: "2".into() },
        ];
        assert_eq!(find_by_slug(&stations, "b").unwrap().video_id, "2");
        assert!(find_by_slug(&stations, "c").is_none());
    }
}
