//! Identity mapping and error conversion helpers for the Feishu plugin.

use closeclaw_common::AdapterError as CommonAdapterError;

use crate::error::AdapterError;

/// Convert im_adapter error to common error.
pub(super) fn convert_to_common_error(e: AdapterError) -> CommonAdapterError {
    match e {
        AdapterError::InvalidPayload(s) => CommonAdapterError::InvalidPayload(s),
        AdapterError::AuthFailed => CommonAdapterError::AuthFailed,
        AdapterError::SendFailed(s) => CommonAdapterError::SendFailed(s),
        AdapterError::InvalidSignature => CommonAdapterError::InvalidSignature,
        AdapterError::IoError(e) => CommonAdapterError::IoError(e),
        AdapterError::UnsupportedOperation => CommonAdapterError::UnsupportedOperation,
    }
}
