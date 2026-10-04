//! Crate-internal helpers shared by unit test modules (`#[cfg(test)]` only).

use crate::retry::CooldownManager;
use std::sync::Arc;

/// Cooldown manager isolated from the real home dir (TempDir-backed persist path).
///
/// The returned `TempDir` must be kept alive for as long as the manager is used.
pub(crate) fn isolated_cooldown() -> (tempfile::TempDir, Arc<CooldownManager>) {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let manager = CooldownManager::with_path(dir.path().join("llm_cooldowns.json"));
    (dir, Arc::new(manager))
}
