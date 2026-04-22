use crate::stations::{extract_livestreams_tab_data, parse_livestreams_tab, Station};
use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use std::time::Duration;

pub const LOFIGIRL_CHANNEL_ID: &str = "UCSJ4gkVC6NrvII8umztf0Ow";

pub const PIPED_INSTANCES: &[&str] = &[
    "api.piped.private.coffee",
    "pipedapi.kavin.rocks",
    "piped-api.garudalinux.org",
    "api.piped.yt",
];

pub fn fetch_stations() -> Result<Vec<Station>> {
    let client = Client::builder().timeout(Duration::from_secs(10)).build()?;
    let mut last_err = anyhow!("no Piped instances available");
    for &instance in PIPED_INSTANCES {
        match fetch_stations_from(&client, instance) {
            Ok(stations) if !stations.is_empty() => return Ok(stations),
            Ok(_) => last_err = anyhow!("{} returned no live stations", instance),
            Err(e) => last_err = anyhow!("{}: {}", instance, e),
        }
    }
    Err(last_err)
}

fn fetch_stations_from(client: &Client, instance: &str) -> Result<Vec<Station>> {
    let channel_url = format!("https://{}/channel/{}", instance, LOFIGIRL_CHANNEL_ID);
    let channel_body = get_text(client, &channel_url, &[])?;
    let tab_data = extract_livestreams_tab_data(&channel_body)?;
    let tab_url = format!("https://{}/channels/tabs", instance);
    let tab_body = get_text(client, &tab_url, &[("data", tab_data.as_str())])?;
    parse_livestreams_tab(&tab_body)
}

fn get_text(client: &Client, url: &str, query: &[(&str, &str)]) -> Result<String> {
    let resp = client.get(url).query(query).send()?;
    if !resp.status().is_success() {
        return Err(anyhow!("returned {}", resp.status()));
    }
    Ok(resp.text()?)
}

pub fn resolve_hls_url(video_id: &str) -> Result<String> {
    let client = Client::builder().timeout(Duration::from_secs(8)).build()?;
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

    // Regression: the previous instance list was entirely dead (two with
    // failing DNS, one returning 502). Keep a currently-working instance
    // pinned at the head so `cargo run` has a chance of succeeding.
    #[test]
    fn instance_list_includes_known_working() {
        assert!(
            PIPED_INSTANCES.contains(&"api.piped.private.coffee"),
            "api.piped.private.coffee must remain in PIPED_INSTANCES until a replacement is verified"
        );
        assert!(!PIPED_INSTANCES.is_empty());
    }
}
