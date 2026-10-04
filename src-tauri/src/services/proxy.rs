//! Upstream HTTP execution: validation, per-hop SSRF re-check, timeouts.

use crate::error::BridgeError;
use crate::services::security;
use reqwest::header::{HeaderValue, ACCEPT, CONTENT_TYPE, USER_AGENT};
use reqwest::redirect::Policy;
use reqwest::{Client, Method};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

/// pixiClient forwarded the browser's UA; a bare request gets a stable one —
/// several CDNs reject clients without a User-Agent.
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

const MAX_REDIRECTS: u8 = 10;

pub struct UpstreamRequest {
    pub url: String,
    pub method: String,
    pub headers: Option<HashMap<String, String>>,
    pub body: Option<String>,
    pub content_type: Option<String>,
    pub timeout_ms: Option<u64>,
}

impl UpstreamRequest {
    /// GET helper — the shape both playlist fetching and segment proxying need.
    pub fn get(url: String, headers: Option<HashMap<String, String>>) -> Self {
        Self {
            url,
            method: "GET".into(),
            headers,
            body: None,
            content_type: None,
            timeout_ms: None,
        }
    }
}

fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            // Redirects are followed manually so each hop passes assert_public().
            .redirect(Policy::none())
            // Bounded even when the total timeout is disabled (media).
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build the HTTP client")
    })
}

/// Validate, send, follow redirects with browser semantics, return the final
/// response (status/headers/body left intact for the caller to consume).
pub async fn execute(request: &UpstreamRequest) -> Result<reqwest::Response, BridgeError> {
    let mut method = security::validate_method(&request.method)?;
    let mut url = security::validate_url(&request.url)?;
    let mut headers = security::sanitize_request_headers(request.headers.as_ref())?;
    let mut body = security::validate_body(request.body.as_deref())?;

    if !headers.contains_key(USER_AGENT) {
        headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
    }
    headers
        .entry(ACCEPT)
        .or_insert_with(|| HeaderValue::from_static("*/*"));
    if let Some(content_type) = &request.content_type {
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::try_from(content_type.as_str())
                .map_err(|_| BridgeError::invalid_headers("invalid content type"))?,
        );
    }

    let timeout = match request.timeout_ms.unwrap_or(security::DEFAULT_TIMEOUT_MS) {
        // `0` disables the total timeout — media responses may take minutes.
        0 => None,
        ms => Some(Duration::from_millis(ms.min(security::MAX_TIMEOUT_MS))),
    };

    for _ in 0..=MAX_REDIRECTS {
        security::assert_public(&url).await?;
        let mut call = client()
            .request(method.clone(), url.clone())
            .headers(headers.clone());
        if let Some(timeout) = timeout {
            call = call.timeout(timeout);
        }
        if let Some(text) = &body {
            call = call.body(text.clone());
        }
        let response = call.send().await.map_err(send_error)?;
        let status = response.status().as_u16();
        if !matches!(status, 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        let Some(location) = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
        else {
            return Ok(response); // no Location: hand the redirect back as-is
        };
        // Browser rules: 301/302/303 downgrade to GET without a body.
        if matches!(status, 301..=303) && method != Method::GET && method != Method::HEAD {
            method = Method::GET;
            body = None;
        }
        let joined = url
            .join(&location)
            .map_err(|_| BridgeError::invalid_url("redirect location is not a valid URL"))?;
        url = security::validate_url(joined.as_str())?;
    }
    Err(BridgeError::response_error(format!(
        "more than {MAX_REDIRECTS} redirects"
    )))
}

fn send_error(error: reqwest::Error) -> BridgeError {
    if error.is_timeout() {
        BridgeError::timeout("upstream request timed out")
    } else {
        BridgeError::connection(format!("could not reach upstream: {error}"))
    }
}

/// End-to-end proof of the money paths: pass-through proxy, Range (seeking),
/// playlist rewriting and the SSRF guard. Needs network.
#[cfg(test)]
mod network_tests {
    use super::*;
    use crate::services::hls;

    const APPLE_MASTER: &str =
        "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/master.m3u8";

    #[test]
    fn proxy_range_and_playlist() {
        tauri::async_runtime::block_on(async {
            // Pass-through: real upstream, real body.
            let response = execute(&UpstreamRequest::get(
                "https://example.com/".into(),
                None,
            ))
            .await
            .expect("example.com should be reachable");
            assert_eq!(response.status().as_u16(), 200);
            let body = response.text().await.unwrap();
            assert!(body.contains("Example Domain"), "unexpected body");

            // Range ⇒ the <video> element can seek (206 + Content-Range).
            let mut range = UpstreamRequest::get("https://example.com/".into(), None);
            range.headers = Some(HashMap::from([(
                "range".into(),
                "bytes=0-99".into(),
            )]));
            let response = execute(&range).await.expect("ranged request");
            assert_eq!(response.status().as_u16(), 206);
            assert!(
                response
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.starts_with("bytes 0-99/")),
                "content-range missing"
            );

            // HLS: fetch + rewrite, every URI routed through the scheme.
            let playlist = hls::fetch_and_rewrite(APPLE_MASTER, None)
                .await
                .expect("master playlist");
            assert!(playlist.starts_with("#EXTM3U"));
            assert!(
                playlist.contains("/m3u8/segment?url="),
                "URIs were not rewritten: {playlist}"
            );
        });
    }

    #[test]
    fn ssrf_guard_blocks_loopback() {
        tauri::async_runtime::block_on(async {
            for target in [
                "http://127.0.0.1:8765/request",
                "https://localhost/x",
                "http://169.254.169.254/latest/meta-data/",
            ] {
                let url = security::validate_url(target).expect("valid URL");
                let result = crate::services::security::assert_public(&url).await;
                assert!(result.is_err(), "{target} should be blocked");
                assert_eq!(result.unwrap_err().error, "BLOCKED_DESTINATION");
            }
        });
    }
}
