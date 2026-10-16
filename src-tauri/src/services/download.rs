//! Saving streams to disk — the app's entire download surface, deliberately
//! narrow: the frontend never picks a path, it gets told which file appeared.
//!
//! ponytail: no progress events and no save-as dialog (fixed Downloads folder,
//! names de-duplicated in place). Upgrade path: emit a per-chunk
//! `download-progress` event for the button, or a file dialog for `save_as`.
//!
//! HLS handling is byte concatenation — init segment + fragments for fMP4,
//! whole segments for MPEG-TS. Encrypted (`EXT-X-KEY`) and byte-ranged
//! (`EXT-X-BYTERANGE`) playlists are rejected with a clear error: those need
//! decryption/demuxing, not joining.

use crate::error::BridgeError;
use crate::services::{proxy, security};
use percent_encoding::percent_decode_str;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_opener::OpenerExt;
use tokio::io::AsyncWriteExt;
use url::Url;

/// Media by URL shape, for CDNs that answer `application/octet-stream`.
const MEDIA_EXTENSIONS: &[&str] = &[".mp4", ".webm", ".mkv", ".mov", ".m4v", ".mp3", ".m4a"];

/// The stream the save service persists: whatever was last handed to the player.
static CURRENT: Mutex<Option<String>> = Mutex::new(None);

/// Remember what is playing. Logs only when the URL actually changes; the
/// `stream-served` event is re-announced on every media response and currently
/// has no listener (the Save button it armed is gone) — kept as the hook a
/// player-side affordance would latch onto. `ponytail:` drop the emit when that
/// affordance is either wired or dropped for good.
pub fn remember<R: Runtime>(app: &AppHandle<R>, url: &str) {
    if let Ok(mut slot) = CURRENT.lock() {
        if slot.as_deref() != Some(url) {
            *slot = Some(url.to_string());
            println!("[stream] ready to save: {url}");
        }
    }
    let _ = app.emit("stream-served", ());
}

/// Is this response worth offering as a download? Used by the protocol handler
/// so JSON/API traffic through `/request` never counts as "a stream".
pub fn is_media_target(url: &str, content_type: &str, ranged: bool) -> bool {
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    ranged
        || content_type.starts_with("video/")
        || content_type.starts_with("audio/")
        || content_type.contains("mpegurl")
        || MEDIA_EXTENSIONS.iter().any(|ext| path.ends_with(ext))
}

/// Save the remembered stream into the user's Downloads folder; returns the
/// file that was written (a frontend affordance would reveal it on demand).
/// No caller in the UI today — see the note in AGENTS.md.
pub async fn save_current<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, BridgeError> {
    let target = CURRENT.lock().ok().and_then(|slot| slot.clone());
    let Some(target) = target else {
        return Err(BridgeError::not_found("No stream has been played yet."));
    };
    let response = get(&target).await?;
    let content_type = header(&response, "content-type");
    let downloads = app.path().download_dir().map_err(internal)?;
    if is_playlist(&target, &content_type) {
        save_hls(&target, response, &downloads).await
    } else {
        save_progressive(&target, response, &content_type, &downloads).await
    }
}

/// Reveal a saved file in the system file manager — only inside Downloads,
/// the one folder this app ever writes to.
pub fn reveal<R: Runtime>(app: &AppHandle<R>, path: &str) -> Result<(), BridgeError> {
    let downloads = app.path().download_dir().map_err(internal)?;
    let downloads = downloads.canonicalize().map_err(internal)?;
    let found = PathBuf::from(path)
        .canonicalize()
        .map_err(|_| BridgeError::not_found("That file no longer exists."))?;
    if !found.starts_with(&downloads) {
        return Err(BridgeError::blocked(
            "Only files in Downloads can be revealed.",
        ));
    }
    app.opener()
        .reveal_item_in_dir(&found)
        .map_err(internal)
}

/// Validated GET with no time ceiling — downloads set their own pace.
/// ponytail: also no idle timeout and no cancel, so a server that stops
/// sending leaves the button spinning until the window closes. Upgrade path:
/// a `tokio_util::sync::CancellationToken` parked in a static, plus a
/// `cancel_download` command wired to the button.
async fn get(target: &str) -> Result<reqwest::Response, BridgeError> {
    let mut request = proxy::UpstreamRequest::get(target.to_string(), None);
    request.timeout_ms = Some(0);
    let response = proxy::execute(&request).await?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(BridgeError::from_upstream(status));
    }
    Ok(response)
}

async fn save_progressive(
    target: &str,
    mut response: reqwest::Response,
    content_type: &str,
    downloads: &Path,
) -> Result<PathBuf, BridgeError> {
    let url = security::validate_url(target)?;
    let path = unique(downloads.join(filename(&url, content_type)));
    let part = path.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await.map_err(internal)?;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| BridgeError::response_error(err.to_string()))?
    {
        file.write_all(&chunk).await.map_err(internal)?;
    }
    commit(file, &part, &path).await?;
    Ok(path)
}

async fn save_hls(
    target: &str,
    response: reqwest::Response,
    downloads: &Path,
) -> Result<PathBuf, BridgeError> {
    let url = security::validate_url(target)?;
    let mut playlist = text(response).await?;
    let mut base = url.clone();

    // Master playlist → follow the highest-bandwidth variant.
    if playlist.lines().any(|line| line.starts_with("#EXT-X-STREAM-INF")) {
        let variant = best_variant(&playlist)
            .ok_or_else(|| BridgeError::response_error("The master playlist has no variants."))?;
        base = base
            .join(variant)
            .map_err(|_| BridgeError::invalid_url("A variant URL is not absolute."))?;
        playlist = text(get(base.as_str()).await?).await?;
    }

    let (init, segments) = plan(&playlist, &base)?;

    let extension = if init.is_some() { ".mp4" } else { ".ts" };
    let path = unique(downloads.join(with_extension(&filename(&url, ""), extension)));
    let part = path.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await.map_err(internal)?;
    if let Some(init_url) = init {
        file.write_all(&get(init_url.as_str()).await?.bytes().await.map_err(internal)?)
            .await
            .map_err(internal)?;
    }
    for segment in segments {
        let bytes = get(segment.as_str()).await?.bytes().await.map_err(internal)?;
        file.write_all(&bytes).await.map_err(internal)?;
    }
    commit(file, &part, &path).await?;
    Ok(path)
}

/// Flush, close, then promote the `.part` file: a save that dies halfway must
/// never sit at the final name looking complete.
async fn commit(mut file: tokio::fs::File, part: &Path, path: &Path) -> Result<(), BridgeError> {
    file.flush().await.map_err(internal)?;
    drop(file);
    tokio::fs::rename(part, path).await.map_err(internal)?;
    Ok(())
}

/// What to fetch, in order: optional init segment, then the media segments.
/// Refuses playlists whose bytes would need decrypting or range assembly —
/// that is demuxing work, not joining.
fn plan(playlist: &str, base: &Url) -> Result<(Option<Url>, Vec<Url>), BridgeError> {
    if playlist
        .lines()
        .any(|line| line.starts_with("#EXT-X-KEY:") && !line.contains("METHOD=NONE"))
    {
        return Err(BridgeError::response_error(
            "Encrypted HLS streams cannot be saved yet.",
        ));
    }
    if playlist.lines().any(|line| line.starts_with("#EXT-X-BYTERANGE")) {
        return Err(BridgeError::response_error(
            "Byte-ranged HLS streams cannot be saved yet.",
        ));
    }

    let init = playlist
        .lines()
        .find(|line| line.starts_with("#EXT-X-MAP"))
        .and_then(uri_attribute)
        .map(|uri| base.join(uri))
        .transpose()
        .map_err(|_| BridgeError::invalid_url("The init segment URL is invalid."))?;
    let segments: Vec<Url> = playlist
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .filter_map(|line| base.join(line.trim()).ok())
        .collect();
    if segments.is_empty() {
        return Err(BridgeError::response_error(
            "The playlist has no media segments.",
        ));
    }
    Ok((init, segments))
}

async fn text(response: reqwest::Response) -> Result<String, BridgeError> {
    response
        .text()
        .await
        .map_err(|err| BridgeError::response_error(err.to_string()))
}

fn is_playlist(target: &str, content_type: &str) -> bool {
    let path = target.split('?').next().unwrap_or(target).to_ascii_lowercase();
    path.ends_with(".m3u8") || content_type.contains("mpegurl")
}

fn header(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Highest declared bandwidth in a master playlist (the line after the tag).
fn best_variant(playlist: &str) -> Option<&str> {
    let mut best: Option<(u64, &str)> = None;
    let mut pending: Option<u64> = None;
    for line in playlist.lines() {
        if line.starts_with("#EXT-X-STREAM-INF") {
            pending = Some(
                line.split("BANDWIDTH=")
                    .nth(1)
                    .map(|rest| rest.chars().take_while(char::is_ascii_digit).collect::<String>())
                    .and_then(|digits| digits.parse().ok())
                    .unwrap_or(0),
            );
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            if let Some(bandwidth) = pending {
                if best.is_none_or(|(peak, _)| bandwidth > peak) {
                    best = Some((bandwidth, line.trim()));
                }
            }
            pending = None;
        }
    }
    best.map(|(_, variant)| variant)
}

/// The `URI="…"` value of a tag (EXT-X-MAP, EXT-X-KEY, …).
fn uri_attribute(tag: &str) -> Option<&str> {
    let start = tag.find("URI=\"")? + "URI=\"".len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Bounded, filesystem-safe name derived from the target URL.
fn filename(url: &Url, content_type: &str) -> String {
    let last = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or_default();
    let decoded = percent_decode_str(last).decode_utf8_lossy();
    // Keep the name boring: no separators or control characters, capped length.
    let cleaned: String = decoded
        .chars()
        .take(80)
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches(&['.', ' '][..]);
    let mut name = if trimmed.is_empty() || !trimmed.chars().any(|c| c.is_alphanumeric()) {
        format!("pixi-{}", unix_now())
    } else {
        trimmed.to_string()
    };
    if !name.contains('.') {
        name.push_str(extension_for(content_type));
    }
    name
}

fn with_extension(name: &str, extension: &str) -> String {
    let stem = Path::new(name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "stream".into());
    format!("{stem}{extension}")
}

fn extension_for(content_type: &str) -> &'static str {
    match content_type.split(';').next().unwrap_or("").trim() {
        "video/mp4" | "application/mp4" => ".mp4",
        "video/webm" => ".webm",
        "video/x-matroska" => ".mkv",
        "video/quicktime" => ".mov",
        "video/mp2t" => ".ts",
        "audio/mpeg" => ".mp3",
        "audio/mp4" | "audio/aac" => ".m4a",
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" => ".m3u8",
        _ => ".bin",
    }
}

/// Never overwrite: `name.ext` → `name (1).ext` → `name (2).ext` → …
fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "stream".into());
    let extension = path
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    (1..1000)
        .map(|n| dir.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(path)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn internal(err: impl std::fmt::Display) -> BridgeError {
    BridgeError::new("INTERNAL_ERROR", err.to_string(), 500)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    #[test]
    fn names_are_path_safe_and_bounded() {
        let odd = filename(&url("https://cdn.test/we%2Fird%3Fname"), "video/mp4");
        assert!(!odd.contains('/') && !odd.contains('\\'), "{odd}");
        assert_eq!(odd, "we_ird_name.mp4");

        let named = filename(&url("https://cdn.test/My%20Episode%2001.mp4?sig=abc"), "");
        assert_eq!(named, "My Episode 01.mp4");

        let hidden = filename(&url("https://cdn.test/.hidden"), "video/mp4");
        assert!(!hidden.starts_with('.'), "{hidden}");

        let bare = filename(&url("https://cdn.test/stream"), "");
        assert_eq!(bare, "stream.bin");
    }

    #[test]
    fn picks_the_busiest_variant() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nlow.m3u8\n\
                      #EXT-X-STREAM-INF:BANDWIDTH=3000000,CODECS=\"avc1\"\nhigh.m3u8\n";
        assert_eq!(best_variant(master), Some("high.m3u8"));
    }

    #[test]
    fn reads_uri_attributes() {
        assert_eq!(
            uri_attribute("#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"0@0\""),
            Some("init.mp4")
        );
        assert_eq!(uri_attribute("#EXT-X-KEY:METHOD=AES-128"), None);
    }

    #[test]
    fn plans_segments_against_the_playlist_url() {
        let base = url("https://cdn.test/vod/index.m3u8");
        let media = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"1000@0\"\n\
                     #EXTINF:4.0,\na.ts\n#EXTINF:4.0,\n../b.ts\n#EXT-X-ENDLIST\n";
        let (init, segments) = plan(media, &base).unwrap();
        assert_eq!(init.unwrap().as_str(), "https://cdn.test/vod/init.mp4");
        let planned: Vec<&str> = segments.iter().map(|segment| segment.as_str()).collect();
        assert_eq!(
            planned,
            [
                "https://cdn.test/vod/a.ts",
                "https://cdn.test/b.ts", // `..` resolves, like the player sees it
            ]
        );
    }

    #[test]
    fn refuses_playlists_it_cannot_just_join() {
        let base = url("https://cdn.test/vod/index.m3u8");
        let encrypted = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:4,\na.ts\n";
        let error = plan(encrypted, &base).unwrap_err();
        assert!(error.message.contains("Encrypted"), "{error}");

        let ranged = "#EXTM3U\n#EXT-X-BYTERANGE:1000@0\n#EXTINF:4,\na.ts\n";
        assert!(plan(ranged, &base).unwrap_err().message.contains("Byte-ranged"));

        let empty = "#EXTM3U\n#EXT-X-ENDLIST\n";
        assert!(plan(empty, &base)
            .unwrap_err()
            .message
            .contains("no media segments"));
    }

    #[test]
    fn never_overwrites_a_previous_save() {
        let dir = std::env::temp_dir().join(format!("pixi-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // The name comes from the URL every time: second save of the same
        // stream must land next to the first, not on top of it.
        let name = || dir.join("a.mp4");
        std::fs::write(name(), b"x").unwrap();
        assert_eq!(unique(name()), dir.join("a (1).mp4"));
        std::fs::write(dir.join("a (1).mp4"), b"y").unwrap();
        assert_eq!(unique(name()), dir.join("a (2).mp4"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;
    use std::io::Read;

    /// The whole progressive path against a real server: validated GET,
    /// chunked write, a real file in a real folder — checked for a valid mp4
    /// header, not just existence.
    #[test]
    fn saves_a_progressive_stream_to_disk() {
        let dir = std::env::temp_dir().join(format!("pixi-dl-net-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = "https://mdn.github.io/shared-assets/videos/flower.mp4";

        let saved = tauri::async_runtime::block_on(async {
            let response = get(target).await?;
            save_progressive(target, response, "video/mp4", &dir).await
        })
        .unwrap();
        assert_eq!(saved, dir.join("flower.mp4"));

        let mut head = [0u8; 12];
        std::fs::File::open(&saved)
            .unwrap()
            .read_exact(&mut head)
            .unwrap();
        assert_eq!(&head[4..8], b"ftyp", "expected a valid mp4, not a stray body");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
