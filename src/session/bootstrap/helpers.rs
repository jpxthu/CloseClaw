//! Helper utilities for bootstrap protection

use super::types::{BootstrapRegion, BOOTSTRAP_REGION_START};

/// Utility: generate region marker string
pub fn make_bootstrap_marker(file_name: &str, content: &str, is_reinject: bool) -> String {
    let hash = BootstrapRegion::compute_hash(content);
    let char_count = content.chars().count();
    format!(
        "{}file={},hash={},chars={},reinject={}>",
        BOOTSTRAP_REGION_START,
        file_name,
        &hash[..12],
        char_count,
        is_reinject
    )
}
