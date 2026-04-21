use crate::stations::{parse_channel_response, Station};
use anyhow::{anyhow, Result};
use std::time::Duration;

pub const LOFIGIRL_CHANNEL_ID: &str = "UCSJ4gkVC6NrvII8umztf0Ow";

pub const PIPED_INSTANCES: &[&str] = &[
    "pipedapi.kavin.rocks",
    "piped-api.garudalinux.org",
    "api.piped.yt",
];

pub fn fetch_stations() -> Result<Vec<Station>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let mut last_err = anyhow!("no Piped instances available");
    for &instance in PIPED_INSTANCES {
        let url = format!("https://{}/channel/{}", instance, LOFIGIRL_CHANNEL_ID);
        match client.get(&url).send() {
            Ok(resp) if resp.status().is_success() => {
                match resp.text() {
                    Ok(text) => match parse_channel_response(&text) {
                        Ok(stations) if !stations.is_empty() => return Ok(stations),
                        Ok(_) => last_err = anyhow!("{} returned no live stations", instance),
                        Err(e) => last_err = anyhow!("{} parse error: {}", instance, e),
                    },
                    Err(e) => last_err = anyhow!("{} read error: {}", instance, e),
                }
            }
            Ok(resp) => last_err = anyhow!("{} returned {}", instance, resp.status()),
            Err(e) => last_err = anyhow!("{} unreachable: {}", instance, e),
        }
    }
    Err(last_err)
}

pub fn resolve_hls_url(video_id: &str) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    let mut last_err = anyhow!("no Piped instances available");
    for &instance in PIPED_INSTANCES {
        let url = format!("https://{}/streams/{}", instance, video_id);
        match client.get(&url).send() {
            Ok(resp) if resp.status().is_success() => {
                match extract_hls_url(&resp.text()?) {
                    Ok(hls) => return Ok(hls),
                    Err(e) => last_err = e,
                }
            }
            Ok(resp) => last_err = anyhow!("{} returned {}", instance, resp.status()),
            Err(e) => last_err = anyhow!("{} unreachable: {}", instance, e),
        }
    }
    Err(last_err)
}

pub fn extract_hls_url(json: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    v["hls"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("no valid 'hls' field in Piped response"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_hls_url_from_json() {
        let json = r#"{"hls": "https://example.com/stream.m3u8", "title": "Test"}"#;
        assert_eq!(extract_hls_url(json).unwrap(), "https://example.com/stream.m3u8");
    }

    #[test]
    fn missing_hls_field_is_error() {
        let json = r#"{"title": "no hls here"}"#;
        assert!(extract_hls_url(json).is_err());
    }

    #[test]
    fn null_hls_field_is_error() {
        let json = r#"{"hls": null}"#;
        assert!(extract_hls_url(json).is_err());
    }

    #[test]
    fn empty_hls_string_is_error() {
        let json = r#"{"hls": ""}"#;
        assert!(extract_hls_url(json).is_err());
    }
}
