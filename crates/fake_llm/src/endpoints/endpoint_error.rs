//! Unified error type for endpoint handlers.
//!
//! Replaces the raw `(StatusCode, HeaderMap, String)` tuple used in handler
//! return types to satisfy `clippy::result_large_err`. Provides constructors
//! that encapsulate status-code conversion and `Retry-After` header assembly.

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// Error returned by endpoint handlers.
///
/// Wraps HTTP status, optional `Retry-After` delay, and an error message body.
/// Implements [`IntoResponse`] to produce an HTTP response with the correct
/// status code, body, and `Retry-After` header when applicable.
#[derive(Debug)]
pub struct EndpointError {
    pub status: StatusCode,
    pub retry_after: Option<u64>,
    pub message: String,
}

impl IntoResponse for EndpointError {
    fn into_response(self) -> Response {
        let mut headers = HeaderMap::new();
        if let Some(secs) = self.retry_after {
            if let Ok(val) = HeaderValue::try_from(secs.to_string()) {
                headers.insert(axum::http::header::RETRY_AFTER, val);
            }
        }
        (self.status, headers, self.message).into_response()
    }
}

impl EndpointError {
    /// Internal server error (500) with no `Retry-After` header.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            retry_after: None,
            message: message.into(),
        }
    }

    /// Custom HTTP error with optional `Retry-After` header.
    ///
    /// `status` is a raw u16; values that do not map to a valid HTTP status
    /// code fall back to 500 Internal Server Error.
    pub fn http(status: u16, retry_after: Option<u64>, message: impl Into<String>) -> Self {
        let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        Self {
            status: code,
            retry_after,
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

    #[tokio::test]
    async fn http_retry_after_zero_still_present() {
        let err = EndpointError::http(429, Some(0), "too fast");
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::from_u16(429).unwrap());
        let retry = resp
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .expect("Retry-After header should be present even when value is 0");
        assert_eq!(retry, "0");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"too fast");
    }

    #[tokio::test]
    async fn http_retry_after_max_u64_valid() {
        let err = EndpointError::http(503, Some(u64::MAX), "overloaded");
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::from_u16(503).unwrap());
        let expected = u64::MAX.to_string();
        let retry = resp
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .expect("Retry-After header should be present for u64::MAX");
        assert_eq!(retry, expected.as_str());
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"overloaded");
    }

    #[tokio::test]
    async fn http_invalid_status_preserves_retry_after() {
        let err = EndpointError::http(9999, Some(42), "bad but retryable");
        let resp = err.into_response();
        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid status 9999 should fall back to 500"
        );
        let retry = resp
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .expect("Retry-After header should survive invalid status fallback");
        assert_eq!(retry, "42");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"bad but retryable");
    }

    #[test]
    fn endpoint_error_debug_format() {
        let err = EndpointError::internal("test");
        let dbg = format!("{err:?}");
        assert!(dbg.contains("EndpointError"));
        assert!(dbg.contains("500"));
        assert!(dbg.contains("test"));
    }
}
