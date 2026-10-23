//! AdapterError — common error types for IM adapters

use closeclaw_common::AdapterError as CommonAdapterError;

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("Invalid payload: {0}")]
    InvalidPayload(String),

    #[error("Authentication failed")]
    AuthFailed,

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Unsupported operation")]
    UnsupportedOperation,
}

impl From<AdapterError> for CommonAdapterError {
    fn from(e: AdapterError) -> Self {
        match e {
            AdapterError::InvalidPayload(s) => CommonAdapterError::InvalidPayload(s),
            AdapterError::AuthFailed => CommonAdapterError::AuthFailed,
            AdapterError::SendFailed(s) => CommonAdapterError::SendFailed(s),
            AdapterError::InvalidSignature => CommonAdapterError::InvalidSignature,
            AdapterError::IoError(e) => CommonAdapterError::IoError(e),
            AdapterError::UnsupportedOperation => CommonAdapterError::UnsupportedOperation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_invalid_payload_maps_variant_and_payload() {
        let err = CommonAdapterError::from(AdapterError::InvalidPayload("bad payload".into()));
        match err {
            CommonAdapterError::InvalidPayload(s) => assert_eq!(s, "bad payload"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_from_auth_failed_maps_variant() {
        let err = CommonAdapterError::from(AdapterError::AuthFailed);
        assert!(matches!(err, CommonAdapterError::AuthFailed));
    }

    #[test]
    fn test_from_send_failed_maps_variant_and_payload() {
        let err = CommonAdapterError::from(AdapterError::SendFailed("timeout".into()));
        match err {
            CommonAdapterError::SendFailed(s) => assert_eq!(s, "timeout"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_from_invalid_signature_maps_variant() {
        let err = CommonAdapterError::from(AdapterError::InvalidSignature);
        assert!(matches!(err, CommonAdapterError::InvalidSignature));
    }

    #[test]
    fn test_from_io_error_maps_variant_and_payload() {
        let err = CommonAdapterError::from(AdapterError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file gone",
        )));
        match err {
            CommonAdapterError::IoError(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
                assert_eq!(e.to_string(), "file gone");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn test_from_unsupported_operation_maps_variant() {
        let err = CommonAdapterError::from(AdapterError::UnsupportedOperation);
        assert!(matches!(err, CommonAdapterError::UnsupportedOperation));
    }

    #[test]
    fn test_from_preserves_payload_content_unchanged() {
        let payloads = ["", "ok", "含中文 payload 🎉\n多行\t含转义"];
        for payload in payloads {
            let err = CommonAdapterError::from(AdapterError::InvalidPayload(payload.to_string()));
            match err {
                CommonAdapterError::InvalidPayload(s) => assert_eq!(s, payload),
                other => panic!("unexpected variant: {other:?}"),
            }
            let err = CommonAdapterError::from(AdapterError::SendFailed(payload.to_string()));
            match err {
                CommonAdapterError::SendFailed(s) => assert_eq!(s, payload),
                other => panic!("unexpected variant: {other:?}"),
            }
        }
    }
}
