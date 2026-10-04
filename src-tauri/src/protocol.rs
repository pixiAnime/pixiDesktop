//! Custom URI-scheme handler — the streaming transport: pixiClient's three
//! routes served by Rust instead of a localhost socket.
//!
//!   GET /request?url=…        pass-through proxy (Range ⇒ seeking preserved)
//!   GET /m3u8?url=…           playlist, rewritten against its own URL
//!   GET /m3u8/segment?url=…   segment (nested playlists re-rewritten)
//!
//! Every response carries its own CORS: hls.js XHRs this scheme from the
//! document, and both WebKit and Chromium treat scheme origins as opaque.

use crate::error::BridgeError;
use crate::services::{download, hls, proxy, security};
use std::collections::HashMap;
use tauri::http::{header, Method, Response};
use tauri::{AppHandle, Runtime};

pub const SCHEME: &str = "pixi";

/// Windows and Android can only intercept `http://<scheme>.localhost/…`;
/// every other platform registers the scheme itself. This is the single source
/// of truth for the URL shape — `protocol_base` hands it to the frontend.
pub fn base() -> &'static str {
    if cfg!(windows) {
        "http://pixi.localhost"
    } else {
        "pixi://localhost"
    }
}

pub fn segment_base() -> String {
    format!("{}/m3u8/segment", base())
}

pub fn handle<R: Runtime>(
    ctx: tauri::UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query = request.uri().query().unwrap_or_default().to_string();
    let incoming = request.headers().clone();
    tauri::async_runtime::spawn(async move {
        responder.respond(route(&app, &method, &path, &query, &incoming).await);
    });
}

async fn route<R: Runtime>(
    app: &AppHandle<R>,
    method: &Method,
    path: &str,
    query: &str,
    incoming: &header::HeaderMap,
) -> Response<Vec<u8>> {
    // The only observable signal from the webview while testing (stdout) —
    // logged before the OPTIONS branch so preflight traffic shows up too.
    println!("[protocol] {method} {path} {query}");
    if method == Method::OPTIONS {
        return preflight(incoming);
    }
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    match path {
        "/request" if method == Method::GET || method == Method::HEAD => {
            passthrough(app, &params, incoming).await
        }
        "/m3u8" => playlist(app, &params).await,
        "/m3u8/segment" => segment(&params, incoming).await,
        _ => error_response(BridgeError::not_found(format!("unknown route: {path}"))),
    }
}

async fn passthrough<R: Runtime>(
    app: &AppHandle<R>,
    params: &HashMap<String, String>,
    incoming: &header::HeaderMap,
) -> Response<Vec<u8>> {
    let Ok(url) = target(params) else {
        return error_response(BridgeError::invalid_url("missing url parameter"));
    };
    let mut request = proxy::UpstreamRequest::get(url.clone(), Some(forwarded(incoming)));
    // No total timeout: a media response may legitimately take minutes.
    request.timeout_ms = Some(0);
    match proxy::execute(&request).await {
        Ok(response) => {
            if (200..300).contains(&response.status().as_u16()) {
                let ranged = response.headers().contains_key("content-range");
                if download::is_media_target(&url, &content_type_of(&response), ranged) {
                    download::remember(app, &url);
                }
            }
            read_bytes(response).await
        }
        Err(error) => error_response(error),
    }
}

async fn playlist<R: Runtime>(
    app: &AppHandle<R>,
    params: &HashMap<String, String>,
) -> Response<Vec<u8>> {
    let Ok(url) = target(params) else {
        return error_response(BridgeError::invalid_url("missing url parameter"));
    };
    match hls::fetch_and_rewrite(&url, None).await {
        Ok(text) => {
            download::remember(app, &url);
            build(200, playlist_headers(), text.into_bytes())
        }
        Err(error) => error_response(error),
    }
}

async fn segment(
    params: &HashMap<String, String>,
    incoming: &header::HeaderMap,
) -> Response<Vec<u8>> {
    let Ok(url) = target(params) else {
        return error_response(BridgeError::invalid_url("missing url parameter"));
    };
    let response = match proxy::execute(&proxy::UpstreamRequest::get(
        url.clone(),
        Some(forwarded(incoming)),
    ))
    .await
    {
        Ok(response) => response,
        Err(error) => return error_response(error),
    };
    if !is_playlist(&response, &url) {
        return read_bytes(response).await;
    }
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return error_response(BridgeError::from_upstream(status));
    }
    let text = match response.text().await {
        Ok(text) => text,
        Err(error) => return error_response(BridgeError::response_error(error.to_string())),
    };
    match security::validate_url(&url) {
        Ok(base) => build(200, playlist_headers(), hls::rewrite(&text, &base).into_bytes()),
        Err(error) => error_response(error),
    }
}

/// A nested playlist must be rewritten against *its* URL, not the caller's.
fn is_playlist(response: &reqwest::Response, url: &str) -> bool {
    content_type_of(response).contains("mpegurl")
        || url
            .split('?')
            .next()
            .is_some_and(|path| path.to_ascii_lowercase().ends_with(".m3u8"))
}

fn content_type_of(response: &reqwest::Response) -> String {
    response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

async fn read_bytes(response: reqwest::Response) -> Response<Vec<u8>> {
    let status = response.status().as_u16();
    let headers = security::response_headers(response.headers());
    // ponytail: Tauri's responder takes `Cow<[u8]>`, so one upstream response
    // lives in RAM until it is handed over — bounded by the Range the webview
    // asks for, but a server that ignores Range pushes the whole file through
    // memory. Upgrade path: serve media from a loopback HTTP server instead.
    match response.bytes().await {
        Ok(body) => build(status, headers, body.to_vec()),
        Err(error) => error_response(BridgeError::response_error(format!(
            "reading the upstream body failed: {error}"
        ))),
    }
}

/// CORS preflight for requests with custom headers (hls.js xhrSetup etc.).
fn preflight(incoming: &header::HeaderMap) -> Response<Vec<u8>> {
    let allowed = incoming
        .get("access-control-request-headers")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("*")
        .to_string();
    build(
        204,
        HashMap::from([
            ("access-control-allow-headers".to_string(), allowed),
            (
                "access-control-allow-methods".to_string(),
                "GET, HEAD, POST, OPTIONS".to_string(),
            ),
            ("access-control-max-age".to_string(), "86400".to_string()),
        ]),
        Vec::new(),
    )
}

/// The browser headers the media pipeline depends on (`range` ⇒ seeking).
fn forwarded(incoming: &header::HeaderMap) -> HashMap<String, String> {
    const KEEP: &[&str] = &[
        "range",
        "if-none-match",
        "if-modified-since",
        "if-range",
        "if-match",
        "accept",
        "accept-language",
        "user-agent",
        "referer",
        "origin",
    ];
    let mut out = HashMap::new();
    for name in KEEP {
        if let Some(value) = incoming.get(*name).and_then(|value| value.to_str().ok()) {
            out.insert((*name).to_string(), value.to_string());
        }
    }
    out
}

fn target(params: &HashMap<String, String>) -> Result<String, BridgeError> {
    params
        .get("url")
        .cloned()
        .ok_or_else(|| BridgeError::invalid_url("missing url parameter"))
}

fn playlist_headers() -> HashMap<String, String> {
    HashMap::from([(
        "content-type".to_string(),
        "application/vnd.apple.mpegurl".to_string(),
    )])
}

fn error_response(error: BridgeError) -> Response<Vec<u8>> {
    let status = error.status;
    let body = serde_json::to_vec(&error).unwrap_or_default();
    build(
        status,
        HashMap::from([("content-type".to_string(), "application/json".to_string())]),
        body,
    )
}

fn build(status: u16, headers: HashMap<String, String>, body: Vec<u8>) -> Response<Vec<u8>> {
    let mut builder = Response::builder().status(status);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    // Added last (and only here — upstream CORS headers are dropped when the
    // response is copied) so the document origin can always read the media.
    builder = builder
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            "accept-ranges, content-length, content-range, content-type, etag, last-modified",
        );
    // Header names/values come from a parsed HeaderMap or literals: cannot fail.
    builder
        .body(body)
        .unwrap_or_else(|_| Response::new(b"internal error".to_vec()))
}
