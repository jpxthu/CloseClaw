//! Unit tests for the startup recovery scan aggregation
//! [`Daemon::run_startup_recovery_scan`] (extracted from
//! `init_phase_3_core_services`, issue #3286 Step 1.3).
//!
//! Behavior dimensions (plan Step 1.5):
//! 1. Boundary: empty storage → both lists empty, no state change.
//! 2. State transition: a session stuck in `migrating` status (crash
//!    between archive Step A and Step C) **without** pending operations →
//!    the scan completes the interrupted archive (migrating → archived)
//!    while dirty/migrated stay empty — nothing to re-push and no stale
//!    key-registry rebuild input.
//!
//! Deliberately not covered here — the dirty-session dimension (a session
//! with pending operations landing in the returned re-push list) is not
//! reproducible against the production [`SqliteStorage`]: the sqlite
//! backend does not persist `pending_operations` (save drops the field,
//! load always returns an empty vec — see `archive_support.rs`), so a
//! dirty session cannot be seeded through any public storage API. That
//! dimension is covered at the closeclaw-session level
//! (`crates/session/src/crash_recovery_tests.rs` and
//! `recovery_migrating_tests.rs`, MemoryStorage-backed) and by the daemon
//! full-path regression suite.

use std::sync::Arc;

use closeclaw_session::persistence::{PersistenceService, SessionCheckpoint, SessionStatus};
use closeclaw_session::storage::SqliteStorage;

use crate::Daemon;

/// Minimal active checkpoint (no routing fields — the scan under test
/// never consults them for these scenarios; agent_id stays `None` so the
/// workflow-recovery injection path touches no per-agent workspace).
fn make_checkpoint(session_id: &str) -> SessionCheckpoint {
    SessionCheckpoint::new(session_id.to_string())
}

// ── Boundary: empty storage ──────────────────────────────────────────────

/// Fresh sqlite storage (no sessions in any status) → the scan returns
/// empty dirty/migrated lists and leaves all three status partitions
/// empty: no recovery action is triggered.
#[tokio::test]
async fn test_startup_recovery_scan_empty_storage_returns_empty_lists() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let storage = Arc::new(SqliteStorage::new(tmp.path()).expect("sqlite storage"));

    let (dirty, migrated) = Daemon::run_startup_recovery_scan(&storage).await;

    assert!(dirty.is_empty(), "no sessions → no dirty sessions");
    assert!(migrated.is_empty(), "no sessions → no migrated sessions");
    assert!(
        storage.list_active_sessions().await.unwrap().is_empty(),
        "scan must not create sessions"
    );
    assert!(
        storage.list_archived_sessions().await.unwrap().is_empty(),
        "scan must not archive anything on empty storage"
    );
    assert!(
        storage.list_migrating_sessions().await.unwrap().is_empty(),
        "scan must not leave migrating rows on empty storage"
    );
}

// ── State transition: interrupted archive completed by the scan ─────────

/// A session whose row is stuck in `migrating` (crash between archive
/// Step A and Step C) and carries no pending operations is completed to
/// `archived` by the scan; active sessions are left untouched. With no
/// pending operations anywhere, dirty and migrated lists stay empty —
/// the registry rebuild that follows the scan receives no stale-registry
/// input.
#[tokio::test]
async fn test_startup_recovery_scan_completes_interrupted_archive() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let storage = Arc::new(SqliteStorage::new(tmp.path()).expect("sqlite storage"));

    // Seed an active session that must survive the scan untouched.
    let active = make_checkpoint("active-1");
    storage.save_checkpoint(&active).await.unwrap();

    // Seed a session left in `migrating`: save as active (writes the
    // transcript into sessions/), then re-save with Migrating status —
    // the exact crash-between-Step-A-and-Step-C intermediate state.
    let mut migrating = make_checkpoint("mig-clean");
    migrating.status = SessionStatus::Active;
    storage.save_checkpoint(&migrating).await.unwrap();
    migrating.status = SessionStatus::Migrating;
    storage.save_checkpoint(&migrating).await.unwrap();
    assert!(
        storage
            .list_migrating_sessions()
            .await
            .unwrap()
            .contains(&"mig-clean".to_string()),
        "precondition: session is in the migrating partition"
    );

    let (dirty, migrated) = Daemon::run_startup_recovery_scan(&storage).await;

    // No pending operations anywhere → nothing to re-push, no
    // migrated-session registry rebuild input.
    assert!(dirty.is_empty(), "no pending ops → no dirty sessions");
    assert!(
        migrated.is_empty(),
        "migrating session without pending ops completes its archive, it is not migrated"
    );

    // The scan completed the interrupted archive: migrating → archived.
    assert!(
        !storage
            .list_migrating_sessions()
            .await
            .unwrap()
            .contains(&"mig-clean".to_string()),
        "migrating row must be resolved by the scan"
    );
    let archived = storage.list_archived_sessions().await.unwrap();
    assert!(
        archived.contains(&"mig-clean".to_string()),
        "interrupted archive must complete to archived, got: {archived:?}"
    );
    let loaded = storage
        .load_archived_checkpoint("mig-clean")
        .await
        .unwrap()
        .expect("archived checkpoint must be loadable after the scan");
    assert_eq!(
        loaded.status,
        SessionStatus::Archived,
        "checkpoint status must reflect the completed archive"
    );

    // The untouched active session stays active and loadable.
    let active_ids = storage.list_active_sessions().await.unwrap();
    assert!(
        active_ids.contains(&"active-1".to_string()),
        "active sessions must be left untouched, got: {active_ids:?}"
    );
    let active_loaded = storage
        .load_checkpoint("active-1")
        .await
        .unwrap()
        .expect("active checkpoint must stay loadable");
    assert_eq!(
        active_loaded.status,
        SessionStatus::Active,
        "active session status must be unchanged by the scan"
    );
}
