//! Request/response validation — ported from pixiClient's validators (docs §5–6)
//! plus a DNS-based SSRF guard. The frontend is untrusted input: everything it
//! asks for is re-checked here, never in JS.

use crate::error::BridgeError;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Method;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use url::Url;

pub const MAX_URL_LEN: usize = 4096;
pub const MAX_HEADERS: usize = 32;
pub const MAX_HEADER_VALUE_LEN: usize = 4096;
pub const MAX_BODY_CHARS: usize = 1_000_000;
pub const MAX_RESPONSE_CHARS: usize = 2_000_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 15_000;
/// Ceiling for a caller-supplied timeout (media passes `0` for "no limit").
pub const MAX_TIMEOUT_MS: u64 = 600_000;

/// Never forwarded upstream: hop-by-hop, plus the transport headers reqwest
/// owns (a client-supplied encoding would leave us handing out bytes we never
/// negotiated, and a stale Host would be wrong after every redirect).
const REQUEST_DENY: &[&str] = &[
    "host",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "accept-encoding",
];

/// Never returned to the frontend: hop-by-hop, cookies, and any upstream CORS
/// (we always answer with our own).
const RESPONSE_DENY: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "set-cookie",
];

/// Absolute http(s) target within the size/character limits pixiWeb enforces.
pub fn validate_url(raw: &str) -> Result<Url, BridgeError> {
    if raw.is_empty() {
        return Err(BridgeError::invalid_url("target URL is empty"));
    }
    if raw.len() > MAX_URL_LEN {
        return Err(BridgeError::invalid_url(format!(
            "target URL exceeds {MAX_URL_LEN} bytes"
        )));
    }
    if raw.contains(&['\r', '\n', '\t'][..]) {
        return Err(BridgeError::invalid_url(
            "target URL contains control characters",
        ));
    }
    let url = Url::parse(raw).map_err(|_| BridgeError::invalid_url("target is not an absolute URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(BridgeError::invalid_url("only http(s) targets are allowed"));
    }
    if url.host().is_none() {
        return Err(BridgeError::invalid_url("target has no host"));
    }
    Ok(url)
}

pub fn validate_method(raw: &str) -> Result<Method, BridgeError> {
    Method::from_bytes(raw.trim().as_bytes())
        .map_err(|_| BridgeError::invalid_method(format!("unsupported method: {raw}")))
}

/// Re-apply the header rules pixiWeb's `sanitizeHeaders` applies in JS.
pub fn sanitize_request_headers(
    raw: Option<&HashMap<String, String>>,
) -> Result<HeaderMap, BridgeError> {
    let mut out = HeaderMap::new();
    let Some(raw) = raw else {
        return Ok(out);
    };
    if raw.len() > MAX_HEADERS {
        return Err(BridgeError::invalid_headers(format!(
            "more than {MAX_HEADERS} headers"
        )));
    }
    for (name, value) in raw {
        if value.len() > MAX_HEADER_VALUE_LEN {
            return Err(BridgeError::invalid_headers(format!(
                "header {name} exceeds {MAX_HEADER_VALUE_LEN} bytes"
            )));
        }
        let lower = name.to_ascii_lowercase();
        if REQUEST_DENY.contains(&lower.as_str()) {
            continue;
        }
        let key = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| BridgeError::invalid_headers(format!("invalid header name: {name}")))?;
        let value = HeaderValue::try_from(value.as_str())
            .map_err(|_| BridgeError::invalid_headers(format!("invalid value for header {lower}")))?;
        out.append(key, value);
    }
    Ok(out)
}

pub fn validate_body(body: Option<&str>) -> Result<Option<String>, BridgeError> {
    match body {
        None => Ok(None),
        Some(text) if text.chars().count() > MAX_BODY_CHARS => Err(BridgeError::too_large(format!(
            "request body exceeds {MAX_BODY_CHARS} characters"
        ))),
        Some(text) => Ok(Some(text.to_string())),
    }
}

/// Upstream response headers the frontend may see (multi-values joined).
pub fn response_headers(headers: &HeaderMap) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    for (name, value) in headers.iter() {
        let key = name.as_str();
        if RESPONSE_DENY.contains(&key) || key.starts_with("access-control-") {
            continue;
        }
        let Ok(value) = value.to_str() else {
            continue;
        };
        out.entry(key.to_string())
            .and_modify(|slot| {
                slot.push_str(", ");
                slot.push_str(value);
            })
            .or_insert_with(|| value.to_string());
    }
    out
}

/// Block loopback/private/link-local/metadata destinations. Checked for every
/// redirect hop — otherwise a public host could bounce us into the LAN.
///
/// ponytail: DNS is resolved twice (here, then again by reqwest) and a
/// TOCTOU window remains between check and connect; cache resolved addrs and
/// pin them per connection only if this ever becomes a real threat model.
pub async fn assert_public(url: &Url) -> Result<(), BridgeError> {
    match url.host() {
        None => Err(BridgeError::invalid_url("target has no host")),
        Some(url::Host::Ipv4(ip)) => check_ip(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => check_ip(IpAddr::V6(ip)),
        Some(url::Host::Domain(domain)) => {
            if domain.eq_ignore_ascii_case("localhost") || !domain.contains('.') {
                return Err(BridgeError::blocked(format!("{domain} is not a public host")));
            }
            let port = url.port_or_known_default().unwrap_or(443);
            let addrs = tokio::net::lookup_host((domain, port))
                .await
                .map_err(|_| BridgeError::connection(format!("DNS lookup failed for {domain}")))?;
            for addr in addrs {
                check_ip(addr.ip())?;
            }
            Ok(())
        }
    }
}

fn check_ip(ip: IpAddr) -> Result<(), BridgeError> {
    if is_blocked(ip) {
        Err(BridgeError::blocked(format!(
            "{ip} is a private, loopback or link-local address"
        )))
    } else {
        Ok(())
    }
}

fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            let [hi, ..] = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (hi & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || (hi & 0xfe00) == 0xfc00 // fc00::/7 unique local
                || v6.to_ipv4_mapped().is_some_and(is_blocked_v4)
        }
    }
}

fn is_blocked_v4(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    v4.is_loopback()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || a == 0 // 0.0.0.0/8
        || a == 10 // 10.0.0.0/8
        || (a == 100 && (64..=127).contains(&b)) // 100.64.0.0/10 CGNAT
        || (a == 172 && (16..=31).contains(&b)) // 172.16.0.0/12
        || (a == 192 && b == 168) // 192.168.0.0/16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_targets() {
        assert!(validate_url("").is_err());
        assert!(validate_url("/relative/path").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("https://x.test/a\r\nHost: evil").is_err());
        assert!(validate_url(&format!("https://x.test/{}", "a".repeat(MAX_URL_LEN))).is_err());
        assert!(validate_url("https://cdn.example/hls/master.m3u8").is_ok());
    }

    #[test]
    fn blocks_private_ranges() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:192.168.0.1",
        ] {
            assert!(is_blocked(ip.parse().unwrap()), "{ip} should be blocked");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700::1111"] {
            assert!(!is_blocked(ip.parse().unwrap()), "{ip} should be allowed");
        }
    }

    #[test]
    fn header_rules_match_pixiweb() {
        let mut raw = HashMap::new();
        raw.insert("X-Token".into(), "ok".into());
        raw.insert("Host".into(), "evil.test".into());
        raw.insert("Accept-Encoding".into(), "br".into());
        let headers = sanitize_request_headers(Some(&raw)).unwrap();
        assert_eq!(headers.get("x-token").unwrap(), "ok");
        assert!(!headers.contains_key("host"));
        assert!(!headers.contains_key("accept-encoding"));

        let mut bad = HashMap::new();
        bad.insert("X-Bad".into(), "a\r\nInjected: 1".into());
        assert!(sanitize_request_headers(Some(&bad)).is_err());
        assert!(sanitize_request_headers(Some(&HashMap::new())).is_ok());
    }
}
