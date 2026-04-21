use anyhow::{anyhow, Result};

pub fn is_master_playlist(text: &str) -> bool {
    text.contains("#EXT-X-STREAM-INF")
}

pub fn parse_master_playlist(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut take_next = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("#EXT-X-STREAM-INF") {
            take_next = true;
        } else if take_next && !line.starts_with('#') && !line.is_empty() {
            urls.push(line.to_string());
            take_next = false;
        }
    }
    urls
}

pub fn parse_media_playlist(text: &str) -> (Vec<String>, bool) {
    let mut urls = Vec::new();
    let mut take_next = false;
    let mut ended = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("#EXTINF") {
            take_next = true;
        } else if line == "#EXT-X-ENDLIST" {
            ended = true;
        } else if take_next && !line.starts_with('#') && !line.is_empty() {
            urls.push(line.to_string());
            take_next = false;
        }
    }
    (urls, ended)
}

pub fn resolve_media_playlist_url(
    manifest_url: &str,
    client: &reqwest::blocking::Client,
) -> Result<String> {
    let text = client.get(manifest_url).send()?.text()?;
    if !is_master_playlist(&text) {
        return Ok(manifest_url.to_string());
    }
    let variant = parse_master_playlist(&text)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("master playlist has no variants"))?;
    if variant.starts_with("http") {
        Ok(variant)
    } else {
        let base = manifest_url.rsplit_once('/').map(|(b, _)| b).unwrap_or(manifest_url);
        Ok(format!("{}/{}", base, variant))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "\
#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=128000\n\
https://cdn.example.com/128k/playlist.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=64000\n\
https://cdn.example.com/64k/playlist.m3u8\n";

    const MEDIA: &str = "\
#EXTM3U\n\
#EXT-X-TARGETDURATION:8\n\
#EXTINF:8.0,\n\
https://cdn.example.com/seg1.aac\n\
#EXTINF:8.0,\n\
https://cdn.example.com/seg2.aac\n";

    const MEDIA_ENDED: &str = "\
#EXTM3U\n\
#EXTINF:8.0,\n\
https://cdn.example.com/seg1.aac\n\
#EXT-X-ENDLIST\n";

    #[test]
    fn detects_master_playlist() {
        assert!(is_master_playlist(MASTER));
        assert!(!is_master_playlist(MEDIA));
    }

    #[test]
    fn parses_master_variant_urls() {
        let urls = parse_master_playlist(MASTER);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "https://cdn.example.com/128k/playlist.m3u8");
    }

    #[test]
    fn parses_media_segment_urls() {
        let (segs, ended) = parse_media_playlist(MEDIA);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], "https://cdn.example.com/seg1.aac");
        assert!(!ended);
    }

    #[test]
    fn detects_end_of_stream() {
        let (_, ended) = parse_media_playlist(MEDIA_ENDED);
        assert!(ended);
    }
}
