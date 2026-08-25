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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn internal_produces_500_empty_headers_and_body() {
        let err = EndpointError::internal("boom");
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let headers = resp.headers();
        // Only the default Content-Type from IntoResponse; no Retry-After
        assert!(
            !headers.contains_key(axum::http::header::RETRY_AFTER),
            "no Retry-After on internal errors"
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"boom");
    }

    #[tokio::test]
    async fn http_valid_status_with_retry_after() {
        let err = EndpointError::http(429, Some(30), "rate limited");
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::from_u16(429).unwrap());
        let retry = resp
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert_eq!(retry, "30");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"rate limited");
    }

    #[tokio::test]
    async fn http_invalid_status_falls_back_to_500() {
        let err = EndpointError::http(9999, None, "bad");
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"bad");
    }

    #[tokio::test]
    async fn http_no_retry_after_header_when_none() {
        let err = EndpointError::http(403, None, "forbidden");
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::from_u16(403).unwrap());
        assert!(
            !resp.headers().contains_key(axum::http::header::RETRY_AFTER),
            "Retry-After header should not be present"
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"forbidden");
    }
}
