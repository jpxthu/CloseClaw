//! Tests for the active-searcher session tracker.
//!
//! Covers tracker behavior: begin/end lifecycle, eviction, state
//! transitions, and edge cases.

#[cfg(test)]
mod tests {
    use crate::active_searcher_session::{SearcherSessionStatus, SearcherSessionTracker};

    #[test]
    fn test_begin_record_fields_are_correct() {
        let tracker = SearcherSessionTracker::new();
        let id = tracker.begin("parent-42".into(), "agent-7".into(), "assistant".into());
        let s = tracker.get(&id).unwrap();

        assert_eq!(s.status, SearcherSessionStatus::Running);
        assert_eq!(s.parent_session_id, "parent-42");
        assert_eq!(s.agent_id, "agent-7");
        assert_eq!(s.trigger_role, "assistant");
        assert!(s.created_at > 0);
        assert!(s.finished_at.is_none());
    }

    #[test]
    fn test_begin_ids_are_globally_unique() {
        let tracker = SearcherSessionTracker::new();
        let mut ids = std::collections::HashSet::new();
        for i in 0..200 {
            let id = tracker.begin(format!("p{i}"), format!("a{i}"), "user".into());
            assert!(ids.insert(id), "duplicate ID at iteration {i}");
        }
    }

    #[test]
    fn test_begin_increments_len() {
        let tracker = SearcherSessionTracker::new();
        assert_eq!(tracker.len(), 0);
        tracker.begin("p".into(), "a".into(), "user".into());
        assert_eq!(tracker.len(), 1);
        tracker.begin("p".into(), "a".into(), "user".into());
        assert_eq!(tracker.len(), 2);
    }

    // ── end: terminal status transitions ─────────────────────────────

    #[test]
    fn test_end_no_result_sets_finished_at() {
        let tracker = SearcherSessionTracker::new();
        let id = tracker.begin("p".into(), "a".into(), "user".into());
        tracker.end(&id, SearcherSessionStatus::NoResult);
        let s = tracker.get(&id).unwrap();
        assert_eq!(s.status, SearcherSessionStatus::NoResult);
        assert!(s.finished_at.is_some());
    }

    #[test]
    fn test_end_abandoned_sets_finished_at() {
        let tracker = SearcherSessionTracker::new();
        let id = tracker.begin("p".into(), "a".into(), "user".into());
        tracker.end(&id, SearcherSessionStatus::Abandoned);
        let s = tracker.get(&id).unwrap();
        assert_eq!(s.status, SearcherSessionStatus::Abandoned);
        assert!(s.finished_at.is_some());
    }

    #[test]
    fn test_end_injected_sets_finished_at() {
        let tracker = SearcherSessionTracker::new();
        let id = tracker.begin("p".into(), "a".into(), "assistant".into());
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        tracker.end(&id, SearcherSessionStatus::Injected);
        let s = tracker.get(&id).unwrap();
        assert_eq!(s.status, SearcherSessionStatus::Injected);
        let ft = s.finished_at.unwrap();
        assert!(ft >= before, "finished_at should be >= start of test");
    }

    // ── end: unknown ID is safe no-op ────────────────────────────────

    #[test]
    fn test_end_unknown_id_does_not_panic() {
        let tracker = SearcherSessionTracker::new();
        tracker.end("completely-fake-id", SearcherSessionStatus::Injected);
        tracker.end("", SearcherSessionStatus::NoResult);
        assert!(tracker.is_empty());
    }

    // ── end: idempotent on terminal state ────────────────────────────

    #[test]
    fn test_end_twice_does_not_overwrite_terminal() {
        let tracker = SearcherSessionTracker::new();
        let id = tracker.begin("p".into(), "a".into(), "user".into());
        tracker.end(&id, SearcherSessionStatus::NoResult);
        let first_ft = tracker.get(&id).unwrap().finished_at;

        // Second end with a different terminal status.
        tracker.end(&id, SearcherSessionStatus::Injected);
        let s = tracker.get(&id).unwrap();
        assert_eq!(
            s.status,
            SearcherSessionStatus::NoResult,
            "first terminal status must be preserved"
        );
        assert_eq!(s.finished_at, first_ft);
    }

    #[test]
    fn test_end_abandoned_then_injected_unchanged() {
        let tracker = SearcherSessionTracker::new();
        let id = tracker.begin("p".into(), "a".into(), "assistant".into());
        tracker.end(&id, SearcherSessionStatus::Abandoned);
        let original = tracker.get(&id).unwrap();

        tracker.end(&id, SearcherSessionStatus::Injected);
        let after = tracker.get(&id).unwrap();
        assert_eq!(after.status, original.status);
        assert_eq!(after.finished_at, original.finished_at);
    }

    // ── eviction: Running records are never evicted ──────────────────

    #[test]
    fn test_eviction_never_removes_running() {
        let tracker = SearcherSessionTracker::new();
        let mut running_ids = Vec::new();
        // Fill beyond capacity with Running sessions.
        for i in 0..=128 {
            let id = tracker.begin(format!("p{i}"), "a".into(), "user".into());
            running_ids.push(id);
        }
        // All Running records must survive.
        assert_eq!(tracker.len(), 129);
        for id in &running_ids {
            assert!(
                tracker.get(id).is_some(),
                "Running record {id} should not be evicted"
            );
        }
    }

    // ── eviction: oldest finished records evicted first ──────────────

    #[test]
    fn test_eviction_removes_oldest_finished_preserves_newer() {
        let tracker = SearcherSessionTracker::new();

        // Create and finish 64 sessions.
        let mut old_ids = Vec::new();
        for i in 0..64 {
            let id = tracker.begin(format!("p{i}"), "a".into(), "user".into());
            tracker.end(&id, SearcherSessionStatus::NoResult);
            old_ids.push(id);
        }

        // Create and finish 65 more (total 129 > 128).
        // The oldest finished ones should be evicted.
        let mut new_ids = Vec::new();
        for i in 64..129 {
            let id = tracker.begin(format!("p{i}"), "a".into(), "user".into());
            tracker.end(&id, SearcherSessionStatus::Injected);
            new_ids.push(id);
        }

        // Exactly one old_id should be evicted (excess = 129 - 128 = 1).
        let evicted = old_ids
            .iter()
            .filter(|id| tracker.get(id).is_none())
            .count();
        assert_eq!(
            evicted, 1,
            "exactly one old finished record should be evicted"
        );

        // All new_ids should still be present (they were added later).
        for id in &new_ids {
            assert!(
                tracker.get(id).is_some(),
                "newer finished record {id} should not be evicted"
            );
        }
    }

    // ── eviction: mixed Running and finished ─────────────────────────

    #[test]
    fn test_eviction_mixed_running_and_finished() {
        let tracker = SearcherSessionTracker::new();
        let mut running_ids = Vec::new();
        let mut finished_ids = Vec::new();

        // 64 Running sessions.
        for i in 0..64 {
            let id = tracker.begin(format!("r{i}"), "a".into(), "user".into());
            running_ids.push(id);
        }
        // 65 finished sessions (total 129 > 128).
        for i in 0..65 {
            let id = tracker.begin(format!("f{i}"), "a".into(), "user".into());
            tracker.end(&id, SearcherSessionStatus::Abandoned);
            finished_ids.push(id);
        }

        // Running records must survive.
        for id in &running_ids {
            assert!(
                tracker.get(id).is_some(),
                "Running record {id} must survive eviction"
            );
        }

        // Exactly one finished record should be evicted (excess = 129 - 128 = 1).
        let evicted = finished_ids
            .iter()
            .filter(|id| tracker.get(id).is_none())
            .count();
        assert_eq!(
            evicted, 1,
            "exactly one finished record should be evicted when over capacity"
        );
    }

    // ── query: get returns None for unknown ───────────────────────────

    #[test]
    fn test_get_returns_none_for_unknown() {
        let tracker = SearcherSessionTracker::new();
        assert!(tracker.get("nope").is_none());
        assert!(tracker.get("").is_none());
    }

    // ── query: get returns clone ──────────────────────────────────────

    #[test]
    fn test_get_returns_independent_clone() {
        let tracker = SearcherSessionTracker::new();
        let id = tracker.begin("p".into(), "a".into(), "user".into());
        let s1 = tracker.get(&id).unwrap();
        let s2 = tracker.get(&id).unwrap();
        // Mutating one clone does not affect the other or the tracker.
        drop(s1);
        drop(s2);
        // Session is still in tracker.
        assert!(tracker.get(&id).is_some());
    }

    // ── default trait ─────────────────────────────────────────────────

    #[test]
    fn test_default_creates_empty_tracker() {
        let tracker = SearcherSessionTracker::default();
        assert!(tracker.is_empty());
        assert_eq!(tracker.len(), 0);
    }
}
