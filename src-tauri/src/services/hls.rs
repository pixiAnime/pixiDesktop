//! M3U8 rewriting — port of pixiClient's PlaylistRewriter (docs §7).
//!
//! Every media reference becomes `<base>/m3u8/segment?url=<absolute>` (also for
//! already-absolute URIs, as pixiClient does), so the webview only ever speaks
//! this app's own scheme: no Blob, no inline playlist, no CORS surprises.
//! Nested playlists are re-rewritten by the handler against their own URL.

use crate::error::BridgeError;
use crate::services::{proxy, security};
use std::collections::HashMap;
use url::Url;

/// Tags whose `URI="…"` attribute points at another resource.
const URI_TAGS: &[&str] = &[
    "#EXT-X-MAP",
    "#EXT-X-KEY",
    "#EXT-X-SESSION-KEY",
    "#EXT-X-MEDIA",
    "#EXT-X-I-FRAME-STREAM-INF",
    "#EXT-X-PART",
    "#EXT-X-PRELOAD-HINT",
    "#EXT-X-RENDITION-REPORT",
    "#EXT-X-SESSION-DATA",
];

/// Rewrite a playlist so every reference resolves through the scheme handler.
/// Newlines are normalized to `\n` (parsers accept either style).
pub fn rewrite(playlist: &str, playlist_url: &Url) -> String {
    let mut out = String::with_capacity(playlist.len() + 64);
    for chunk in playlist.split_inclusive('\n') {
        let (line, newline) = match chunk.strip_suffix('\n') {
            Some(rest) => (rest.strip_suffix('\r').unwrap_or(rest), "\n"),
            None => (chunk, ""),
        };
        out.push_str(&rewrite_line(line, playlist_url));
        out.push_str(newline);
    }
    out
}

fn rewrite_line(line: &str, base: &Url) -> String {
    if line.starts_with('#') {
        return rewrite_uri_attributes(line, base);
    }
    if line.trim().is_empty() {
        return line.to_string();
    }
    // Unresolvable references stay untouched (same fallback as pixiClient).
    resolve(line, base).unwrap_or_else(|| line.to_string())
}

fn rewrite_uri_attributes(line: &str, base: &Url) -> String {
    if !line.contains("URI=\"") || !URI_TAGS.iter().any(|tag| line.starts_with(tag)) {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + 32);
    let mut rest = line;
    while let Some(start) = rest.find("URI=\"") {
        let (head, tail) = rest.split_at(start + "URI=\"".len());
        out.push_str(head);
        match tail.find('"') {
            Some(end) => {
                let value = &tail[..end];
                out.push_str(&resolve(value, base).unwrap_or_else(|| value.to_string()));
                out.push('"');
                rest = &tail[end + 1..];
            }
            None => {
                out.push_str(tail);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Wrap an absolute URL in the scheme's segment route (percent-encoded).
/// Non-http(s) schemes — `data:` keys, `skd:` DRM — are left alone.
fn resolve(reference: &str, base: &Url) -> Option<String> {
    let absolute = base.join(reference.trim()).ok()?;
    if !matches!(absolute.scheme(), "http" | "https") {
        return None;
    }
    let encoded: String = url::form_urlencoded::byte_serialize(absolute.as_str().as_bytes()).collect();
    Some(format!("{}?url={}", crate::protocol::segment_base(), encoded))
}

/// GET a playlist through the validated proxy and rewrite it.
pub async fn fetch_and_rewrite(
    url: &str,
    headers: Option<HashMap<String, String>>,
) -> Result<String, BridgeError> {
    let response = proxy::execute(&proxy::UpstreamRequest::get(url.to_string(), headers)).await?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(BridgeError::from_upstream(status));
    }
    let text = response
        .text()
        .await
        .map_err(|_| BridgeError::response_error("failed to read the playlist body"))?;
    if text.chars().count() > security::MAX_RESPONSE_CHARS {
        return Err(BridgeError::too_large("playlist exceeds the response limit"));
    }
    let base = security::validate_url(url)?;
    Ok(rewrite(&text, &base))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://cdn.example/hls/master.m3u8").unwrap()
    }

    fn seg(absolute: &str) -> String {
        let encoded: String = url::form_urlencoded::byte_serialize(absolute.as_bytes()).collect();
        format!("{}?url={}", crate::protocol::segment_base(), encoded)
    }

    #[test]
    fn rewrites_master_playlist_variants() {
        let playlist = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\n"
            .to_string()
            + "low/index.m3u8\n"
            + "#EXT-X-STREAM-INF:BANDWIDTH=2000000\n"
            + "https://other.example/high.m3u8\n";
        let out = rewrite(&playlist, &base());
        assert!(out.contains(&seg("https://cdn.example/hls/low/index.m3u8")));
        assert!(out.contains(&seg("https://other.example/high.m3u8")));
        assert!(out.contains("#EXT-X-STREAM-INF:BANDWIDTH=800000\n"));
    }

    #[test]
    fn rewrites_segment_lines_and_keeps_comments() {
        let playlist = "#EXTM3U\r\n#EXTINF:4.0,\r\nseg-1.ts?tok=a\r\n\r\n";
        let out = rewrite(playlist, &base());
        assert!(out.contains(&seg("https://cdn.example/hls/seg-1.ts?tok=a")));
        assert!(out.contains("#EXTINF:4.0,"));
        assert!(!out.contains('\r'));
    }

    #[test]
    fn rewrites_key_and_init_segment_attributes() {
        let playlist = "#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x1\n"
            .to_string()
            + "#EXT-X-MAP:URI=\"init.mp4\"\n"
            + "seg.m4s\n";
        let out = rewrite(&playlist, &base());
        assert!(out.contains(&format!("URI=\"{}\"", seg("https://cdn.example/hls/key.bin"))));
        assert!(out.contains(&format!("URI=\"{}\"", seg("https://cdn.example/hls/init.mp4"))));
        assert!(out.contains("IV=0x1"));
    }

    #[test]
    fn leaves_data_uri_keys_alone() {
        let playlist = "#EXT-X-KEY:METHOD=AES-128,URI=\"data:text/plain;base64,S0tDSw==\"\n";
        let out = rewrite(playlist, &base());
        assert!(out.contains("URI=\"data:text/plain;base64,S0tDSw==\""));
    }

    #[test]
    fn ignores_unrelated_hash_lines() {
        let playlist = "#EXT-X-TARGETDURATION:6\n";
        assert_eq!(rewrite(playlist, &base()), playlist);
    }
}
