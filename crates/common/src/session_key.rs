//! Deterministic session key computation.
//!
//! Algorithm (from design doc `inbound-chain.md`):
//!
//! ```text
//! routing_fields = "{channel}:{from}:{to}:{account_id}:{timestamp_ms}"
//! hash           = sha256(routing_fields)
//! session_key    = "{timestamp_ms}-{hash_hex}"
//! ```
//!
//! When `account_id` is `None`, the literal string `"default"` is used.

use sha2::{Digest, Sha256};

/// Compute a session key for the given context.
///
/// The key is deterministic: identical inputs always produce the same key.
/// This allows session resolution across different inbound paths (Gateway,
/// SessionRouter) without a shared session table.
pub fn compute_session_key(
    channel: &str,
    from: &str,
    to: &str,
    account_id: Option<&str>,
    timestamp_ms: i64,
) -> String {
    let acc = account_id.unwrap_or("default");
    let routing_fields = format!("{}:{}:{}:{}:{}", channel, from, to, acc, timestamp_ms);
    let hash = Sha256::digest(routing_fields.as_bytes());
    format!("{}-{:x}", timestamp_ms, hash)
}
