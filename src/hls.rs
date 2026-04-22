use anyhow::{anyhow, Result};
use reqwest::Url;
use std::collections::HashSet;

pub fn is_master_playlist(text: &str) -> bool {
    text.contains("#EXT-X-STREAM-INF")
}

/// Resolve a (possibly relative) HLS reference against the playlist URL that
/// contained it. Handles absolute URLs, root-relative (`/path`) references,
/// and directory-relative references uniformly via `Url::join`.
pub fn join_url(base: &str, reference: &str) -> Result<String> {
    let base = Url::parse(base)?;
    Ok(base.join(reference)?.to_string())
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
    resolve_media_playlist_from_text(manifest_url, &text)
}

/// Parse a media playlist and return the absolute URLs of segments that
/// have not already been played (i.e. are not in `seen`). Returned URLs
/// are joined against `media_url` so callers can both fetch them and
/// insert them back into `seen` in the same form they are compared.
pub fn new_segments_since(
    media_url: &str,
    playlist_text: &str,
    seen: &HashSet<String>,
) -> (Vec<String>, bool) {
    let (raw, ended) = parse_media_playlist(playlist_text);
    let fresh = raw
        .into_iter()
        .map(|u| join_url(media_url, &u).unwrap_or(u))
        .filter(|u| !seen.contains(u))
        .collect();
    (fresh, ended)
}

pub fn resolve_media_playlist_from_text(manifest_url: &str, text: &str) -> Result<String> {
    if !is_master_playlist(text) {
        return Ok(manifest_url.to_string());
    }
    let variant = parse_master_playlist(text)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("master playlist has no variants"))?;
    join_url(manifest_url, &variant)
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

    #[test]
    fn join_url_passes_through_absolute_reference() {
        let joined = join_url(
            "https://host.example/a/b/master.m3u8",
            "https://other.example/x/y.m3u8",
        )
        .unwrap();
        assert_eq!(joined, "https://other.example/x/y.m3u8");
    }

    // Regression: Piped's master playlist returns variant URLs as root-
    // relative paths like `/api/manifest/hls_playlist/...`. The previous
    // implementation joined them as directory-relative, producing URLs
    // that 404'd and stalled the stream on "Connecting".
    #[test]
    fn join_url_resolves_root_relative_reference_against_origin() {
        let joined = join_url(
            "https://proxy.example/api/manifest/hls_variant/expire/1/id/xx/master.m3u8",
            "/api/manifest/hls_playlist/expire/1/id/xx/playlist.m3u8",
        )
        .unwrap();
        assert_eq!(
            joined,
            "https://proxy.example/api/manifest/hls_playlist/expire/1/id/xx/playlist.m3u8"
        );
    }

    #[test]
    fn join_url_resolves_plain_relative_reference_against_directory() {
        let joined = join_url("https://cdn.example/stream/master.m3u8", "segment_001.aac").unwrap();
        assert_eq!(joined, "https://cdn.example/stream/segment_001.aac");
    }

    #[test]
    fn resolve_media_playlist_from_text_returns_input_for_media_playlist() {
        let url = "https://cdn.example/stream/media.m3u8";
        assert_eq!(resolve_media_playlist_from_text(url, MEDIA).unwrap(), url);
    }

    #[test]
    fn resolve_media_playlist_from_text_joins_root_relative_variant() {
        let master = "\
#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=128000\n\
/api/manifest/hls_playlist/id/xx/playlist.m3u8\n";
        let got = resolve_media_playlist_from_text(
            "https://proxy.example/api/manifest/hls_variant/id/xx/master.m3u8",
            master,
        )
        .unwrap();
        assert_eq!(
            got,
            "https://proxy.example/api/manifest/hls_playlist/id/xx/playlist.m3u8"
        );
    }

    #[test]
    fn resolve_media_playlist_from_text_passes_through_absolute_variant() {
        let got =
            resolve_media_playlist_from_text("https://proxy.example/master.m3u8", MASTER).unwrap();
        assert_eq!(got, "https://cdn.example.com/128k/playlist.m3u8");
    }

    #[test]
    fn resolve_media_playlist_from_text_errors_on_empty_master() {
        let empty = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=0\n";
        assert!(resolve_media_playlist_from_text("https://h/m.m3u8", empty).is_err());
    }

    #[test]
    fn new_segments_since_returns_all_when_seen_is_empty() {
        let playlist = "\
#EXTM3U\n\
#EXTINF:5.0,\n\
/seg/1.ts\n\
#EXTINF:5.0,\n\
/seg/2.ts\n";
        let (segs, _) = new_segments_since(
            "https://cdn.example/path/media.m3u8",
            playlist,
            &HashSet::new(),
        );
        assert_eq!(
            segs,
            vec![
                "https://cdn.example/seg/1.ts".to_string(),
                "https://cdn.example/seg/2.ts".to_string(),
            ]
        );
    }

    // Regression: the streaming loop previously tracked seen segments in
    // their absolute (joined) form but compared them against relative
    // URLs from the playlist, so every poll re-selected every segment
    // and the audio rolled back. `new_segments_since` must return and
    // consume the *same* joined form, so round-tripping `seen` makes
    // the next call return an empty list.
    #[test]
    fn new_segments_since_excludes_already_seen_segments() {
        let playlist = "\
#EXTM3U\n\
#EXTINF:5.0,\n\
/seg/1.ts\n\
#EXTINF:5.0,\n\
/seg/2.ts\n";
        let media_url = "https://cdn.example/path/media.m3u8";
        let (first, _) = new_segments_since(media_url, playlist, &HashSet::new());
        let mut seen: HashSet<String> = first.into_iter().collect();
        let (second, _) = new_segments_since(media_url, playlist, &seen);
        assert!(
            second.is_empty(),
            "segments already in `seen` must not be re-emitted (got {:?})",
            second
        );
        // Adding a new segment shows only the new one is emitted.
        let playlist2 = format!("{}#EXTINF:5.0,\n/seg/3.ts\n", playlist);
        let (third, _) = new_segments_since(media_url, &playlist2, &seen);
        assert_eq!(third, vec!["https://cdn.example/seg/3.ts".to_string()]);
        seen.extend(third);
        let (fourth, _) = new_segments_since(media_url, &playlist2, &seen);
        assert!(fourth.is_empty());
    }

    #[test]
    fn new_segments_since_joins_root_relative_and_reports_end_of_stream() {
        let playlist = "\
#EXTM3U\n\
#EXTINF:5.0,\n\
/seg/only.ts\n\
#EXT-X-ENDLIST\n";
        let (segs, ended) = new_segments_since(
            "https://host.example/hls/live.m3u8",
            playlist,
            &HashSet::new(),
        );
        assert_eq!(segs, vec!["https://host.example/seg/only.ts".to_string()]);
        assert!(ended);
    }
}
