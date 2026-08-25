//! Unified error type for endpoint handlers.
//!
//! Replaces the raw `(StatusCode, HeaderMap, String)` tuple used in handler
//! return types to satisfy `clippy::result_large_err`. Provides constructors
//! that encapsulate status-code conversion and `Retry-After` header assembly.

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// Error returned by endpoint handlers.
///
/// Wraps HTTP status, optional response headers, and an error message body.
/// Implements [`IntoResponse`] to produce the same response as the original
/// `(StatusCode, HeaderMap, String)` tuple.
#[derive(Debug)]
pub struct EndpointError {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub message: String,
}

impl IntoResponse for EndpointError {
    fn into_response(self) -> Response {
        (self.status, self.headers, self.message).into_response()
    }
}

impl EndpointError {
    /// Internal server error (500) with an empty header map.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            headers: HeaderMap::new(),
            message: message.into(),
        }
    }

    /// Custom HTTP error with optional `Retry-After` header.
    ///
    /// `status` is a raw u16; values that do not map to a valid HTTP status
    /// code fall back to 500 Internal Server Error.
    pub fn http(status: u16, retry_after: Option<u64>, message: impl Into<String>) -> Self {
        let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut headers = HeaderMap::new();
        if let Some(secs) = retry_after {
            if let Ok(val) = HeaderValue::try_from(secs.to_string()) {
                headers.insert(axum::http::header::RETRY_AFTER, val);
            }
        }
        Self {
            status: code,
            headers,
            message: message.into(),
        }
    }
}
