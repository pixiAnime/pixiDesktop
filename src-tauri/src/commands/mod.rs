//! Typed IPC surface — the frontend's only network path. Every argument is
//! re-validated here with the same rules pixiClient applied (docs §5–6).

use crate::error::BridgeError;
use crate::services::{download, hls, proxy, security};
use serde::Serialize;
use std::collections::HashMap;

/// Pass-through proxy: validates and executes an upstream request, returning
/// status + headers + text (binary payloads go through the scheme handler).
#[tauri::command]
pub async fn http_request(
    url: String,
    method: Option<String>,
    headers: Option<HashMap<String, String>>,
    body: Option<String>,
    content_type: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<HttpResult, BridgeError> {
    let method = method.unwrap_or_else(|| "GET".into());
    let url_log: String = url.chars().take(140).collect();
    let started = std::time::Instant::now();
    let response = proxy::execute(&proxy::UpstreamRequest {
        url,
        method: method.clone(),
        headers,
        body,
        content_type,
        timeout_ms,
    })
    .await?;
    let status = response.status().as_u16();
    // The only trace of command traffic a GUI process leaves — this is how
    // slow or failing upstreams are spotted in a headless run.
    println!(
        "[http] {} {} -> {} ({:.2}s)",
        method,
        url_log,
        status,
        started.elapsed().as_secs_f64()
    );
    let headers = security::response_headers(response.headers());
    let text = response
        .text()
        .await
        .map_err(|_| BridgeError::response_error("failed to read the upstream body"))?;
    if text.chars().count() > security::MAX_RESPONSE_CHARS {
        return Err(BridgeError::too_large(
            "upstream response exceeds the response limit",
        ));
    }
    Ok(HttpResult {
        status,
        headers,
        text,
    })
}

/// Fetch and rewrite an HLS playlist (pixiClient's `/m3u8`). A successful
/// fetch also marks the URL as the stream the save service will persist.
#[tauri::command]
pub async fn fetch_playlist(
    app: tauri::AppHandle,
    url: String,
    headers: Option<HashMap<String, String>>,
) -> Result<String, BridgeError> {
    let text = hls::fetch_and_rewrite(&url, headers).await?;
    download::remember(&app, &url);
    Ok(text)
}

/// Save the current stream into Downloads; resolves with the file path.
#[tauri::command]
pub async fn download_stream(app: tauri::AppHandle) -> Result<String, BridgeError> {
    download::save_current(&app)
        .await
        .map(|path| path.display().to_string())
}

/// Reveal a saved file in the system file manager (Downloads only).
#[tauri::command]
pub fn reveal_item(app: tauri::AppHandle, path: String) -> Result<(), BridgeError> {
    download::reveal(&app, &path)
}

#[derive(Debug, Serialize)]
pub struct HttpResult {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub text: String,
}
