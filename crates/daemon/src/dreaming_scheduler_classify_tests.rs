//! Unit tests for [`classify_config_recv`] — config broadcast receive
//! result classification (issue #3577, plan Step 1.4).
//!
//! Mounted as a child module of `dreaming_scheduler::events` (via
//! `#[path]`) so the private pure function and its shared outcome enum
//! stay directly unit-testable with `use super::*`.

use super::*;

/// Build an `Ok(..)` receive result carrying a `Reloaded` event for the
/// given section.
fn reloaded(section: ConfigSection) -> Result<ConfigChangeEvent, RecvError> {
    Ok(ConfigChangeEvent::Reloaded {
        section,
        path: "memory.json".into(),
    })
}

// ── Normal path: reload ────────────────────────────────────────────────

/// A Memory-section reload — the only event the loop acts on — must
/// classify as `MemoryReload` so the caller applies its reload handler.
#[test]
fn test_classify_memory_reloaded_maps_to_reload() {
    let outcome = classify_config_recv(reloaded(ConfigSection::Memory));
    assert!(
        matches!(outcome, ConfigRecvOutcome::MemoryReload),
        "Ok(Reloaded{{Memory}}) must classify as MemoryReload"
    );
}

// ── Ignore path: non-reload events keep polling ────────────────────────

/// Reloads of any other section must classify as `Ignore`: the event is
/// consumed and the loop keeps polling without applying any handler.
/// Two non-Memory sections assert the mapping is not section-specific.
#[test]
fn test_classify_non_memory_reloaded_maps_to_ignore() {
    for section in [ConfigSection::Gateway, ConfigSection::Session] {
        let outcome = classify_config_recv(reloaded(section));
        assert!(
            matches!(outcome, ConfigRecvOutcome::Ignore),
            "Ok(Reloaded{{section={section:?}}}) must classify as Ignore"
        );
    }
}

/// Boundary: even for the Memory section, a non-`Reloaded` event
/// (`Failed`) is not a reload — classification keys on both the event
/// variant and the section, so failed reloads are silently ignored too.
#[test]
fn test_classify_memory_failed_maps_to_ignore() {
    let failed = Ok(ConfigChangeEvent::Failed {
        section: ConfigSection::Memory,
        path: "memory.json".into(),
        error: "parse error".into(),
    });
    let outcome = classify_config_recv(failed);
    assert!(
        matches!(outcome, ConfigRecvOutcome::Ignore),
        "Ok(Failed{{Memory}}) must classify as Ignore, not MemoryReload"
    );
}

// ── Boundary path: lagged receiver survives ────────────────────────────

/// `Err(RecvError::Lagged(n))` must classify as `Lagged` with the lost
/// count preserved verbatim (the loop warns with that number and keeps
/// polling). End-to-end survival — the loop staying alive and applying
/// a subsequent valid reload after lag — is covered by
/// `test_config_broadcast_lagged_loop_survives_and_applies_reload`.
#[test]
fn test_classify_lagged_preserves_lost_count() {
    // 65 = capacity (64) + 1, the smallest possible lag.
    match classify_config_recv(Err(RecvError::Lagged(65))) {
        ConfigRecvOutcome::Lagged(n) => {
            assert_eq!(n, 65, "the lost-event count must be preserved verbatim");
        }
        _ => panic!("Err(Lagged(65)) must classify as ConfigRecvOutcome::Lagged"),
    }
}

// ── Error path: closed channel terminates ──────────────────────────────

/// `Err(RecvError::Closed)` must classify as `Closed` — the loop exits
/// via the shutdown path.
///
/// End-to-end unreachability: this branch cannot be driven through a
/// running scheduler. `DreamingScheduler` owns an `Arc<ConfigManager>`
/// (field `config_manager`) and subscribes `config_rx` from that same
/// manager, so the broadcast sender is kept alive for as long as the
/// scheduler itself exists and `recv()` can never return
/// `RecvError::Closed` in production (issue #3577 flags the branch as
/// edge-case robustness only). The terminate mapping is therefore
/// covered by this classification unit test; the termination path
/// itself needs no separate end-to-end test because
/// `LoopEvent::ConfigClosed` shares the exact match arm with
/// `LoopEvent::Shutdown` (`finish_shutdown` + break), which is already
/// exercised end to end by the shutdown tests
/// (e.g. `test_dreaming_scheduler_shutdown_exits_loop`).
#[test]
fn test_classify_closed_maps_to_terminate() {
    let outcome = classify_config_recv(Err(RecvError::Closed));
    assert!(
        matches!(outcome, ConfigRecvOutcome::Closed),
        "Err(Closed) must classify as ConfigRecvOutcome::Closed (terminate)"
    );
}
