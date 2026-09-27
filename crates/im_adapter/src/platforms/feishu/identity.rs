//! Identity mapping and error conversion helpers for the Feishu plugin.

use closeclaw_common::identity::IdentityResolver;
use closeclaw_common::AdapterError as CommonAdapterError;
use closeclaw_config::identity::ConfigIdentityResolver;
use std::sync::Arc;
use tracing::info;

use crate::error::AdapterError;

/// Try to load identity mappings from `{config_dir}/config/accounts.json`.
///
/// Returns `Some(Arc<ConfigIdentityResolver>)` when the file exists and
/// contains a valid JSON object with an `accounts` array, or `None` on
/// any error / missing file.
pub(crate) fn load_identity_resolver(config_dir: &str) -> Option<Arc<dyn IdentityResolver>> {
    use closeclaw_config::AccountsConfigData;

    let path = std::path::Path::new(config_dir)
        .join("config")
        .join("accounts.json");
    match std::fs::read_to_string(&path) {
        Ok(json) => match AccountsConfigData::from_json_str(&json) {
            Ok(accounts_data) => {
                let resolver = ConfigIdentityResolver::new(accounts_data.accounts);
                if resolver.is_empty() {
                    info!("accounts.json loaded but empty — no mappings configured");
                    None
                } else {
                    info!(
                        count = resolver.len(),
                        "identity mapping loaded from {}",
                        path.display()
                    );
                    Some(Arc::new(resolver))
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "failed to parse accounts.json — skipping identity mapping"
                );
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!("accounts.json not found — identity mapping disabled");
            None
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "failed to read accounts.json — skipping identity mapping"
            );
            None
        }
    }
}

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
