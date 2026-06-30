//! Additional unit tests for MemoryMiner.
//!
//! Complements the inline tests in dreaming.rs with additional edge cases.

use crate::miner::MemoryMiner;
use crate::test_helpers::TestStorage;
use closeclaw_session::persistence::SessionCheckpoint;

// ── Tests ────────────────────────────────────────────────────────────────

/// Mining a session with a transcript marks it as mined.
#[tokio::test]
async fn test_mine_session_marks_mined_with_transcript() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-transcript".into());
    cp.mined = false;
    storage.add_checkpoint(cp);

    let miner = MemoryMiner::new();
    miner
        .mine_session("sess-transcript", &storage)
        .await
        .unwrap();

    let mined = storage.mined_ids();
    assert!(
        mined.contains(&"sess-transcript".to_string()),
        "session should be marked as mined"
    );
}

/// Mining an already-mined session is a no-op (idempotent).
#[tokio::test]
async fn test_mine_session_idempotent_on_already_mined() {
    let storage = TestStorage::default();
    let mut cp = SessionCheckpoint::new("sess-idempotent".into());
    cp.mined = true;
    storage.add_checkpoint(cp);

    let miner = MemoryMiner::new();
    miner
        .mine_session("sess-idempotent", &storage)
        .await
        .unwrap();

    let mined = storage.mined_ids();
    assert!(
        mined.is_empty(),
        "should not call mark_mined again for already-mined session"
    );
}

/// Mining a nonexistent session returns an error.
#[tokio::test]
async fn test_mine_session_nonexistent_returns_error() {
    let storage = TestStorage::default();
    let miner = MemoryMiner::new();
    let result = miner.mine_session("does-not-exist", &storage).await;
    assert!(result.is_err(), "mining nonexistent session should fail");
}

/// Mining multiple sessions processes each independently.
#[tokio::test]
async fn test_mine_multiple_sessions() {
    let storage = TestStorage::default();
    for i in 0..5 {
        let cp = SessionCheckpoint::new(format!("sess-{i}"));
        storage.add_checkpoint(cp);
    }

    let miner = MemoryMiner::new();
    for i in 0..5 {
        miner
            .mine_session(&format!("sess-{i}"), &storage)
            .await
            .unwrap();
    }

    let mined = storage.mined_ids();
    assert_eq!(mined.len(), 5, "all 5 sessions should be mined");
    for i in 0..5 {
        assert!(
            mined.contains(&format!("sess-{i}")),
            "sess-{i} should be in mined list"
        );
    }
}
