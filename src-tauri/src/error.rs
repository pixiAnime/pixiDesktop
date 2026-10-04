//! Bridge error taxonomy — the exact codes pixiClient returns (docs §6.7), so
//! pixiWeb's `BridgeError` classification and its `PIXICLIENT_UNAVAILABLE`
//! handling keep working unchanged.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct BridgeError {
    pub error: &'static str,
    pub message: String,
    pub status: u16,
}

impl BridgeError {
    pub fn new(error: &'static str, message: impl Into<String>, status: u16) -> Self {
        Self {
            error,
            message: message.into(),
            status,
        }
    }

    pub fn invalid_url(message: impl Into<String>) -> Self {
        Self::new("INVALID_URL", message, 400)
    }
    pub fn invalid_method(message: impl Into<String>) -> Self {
        Self::new("INVALID_METHOD", message, 400)
    }
    pub fn invalid_headers(message: impl Into<String>) -> Self {
        Self::new("INVALID_HEADERS", message, 400)
    }
    pub fn blocked(message: impl Into<String>) -> Self {
        Self::new("BLOCKED_DESTINATION", message, 403)
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new("UNAUTHORIZED", message, 401)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("NOT_FOUND", message, 404)
    }
    pub fn too_large(message: impl Into<String>) -> Self {
        Self::new("REQUEST_TOO_LARGE", message, 413)
    }
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new("CONNECTION_TIMEOUT", message, 504)
    }
    pub fn connection(message: impl Into<String>) -> Self {
        Self::new("UPSTREAM_CONNECTION_FAILED", message, 502)
    }
    pub fn response_error(message: impl Into<String>) -> Self {
        Self::new("RESPONSE_ERROR", message, 502)
    }

    /// How an upstream HTTP status surfaces to the frontend (pixiClient rules:
    /// 401/403 → UNAUTHORIZED, 404/410 → NOT_FOUND, everything else → error).
    pub fn from_upstream(status: u16) -> Self {
        match status {
            401 | 403 => Self::unauthorized(format!("upstream returned {status}")),
            404 | 410 => Self::not_found(format!("upstream returned {status}")),
            _ => Self::response_error(format!("upstream returned {status}")),
        }
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.error, self.message)
    }
}

impl std::error::Error for BridgeError {}
