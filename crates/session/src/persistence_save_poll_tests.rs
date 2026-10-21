//! Deadline-bounded polling for `save()` background writes, shared by
//! persistence tests, plus the timeout-path boundary test.
//!
//! `CheckpointManager::save()` writes to storage in a spawned background
//! task, so the arrival instant is not observable — callers poll the storage
//! with an explicit deadline instead of a single `yield_now` (#3861 family).

use std::time::Duration;

use crate::persistence::PersistenceService;
use crate::storage::memory::MemoryStorage;

/// Poll storage until a checkpoint for `session_id` appears, bounded by an
/// explicit deadline. Returns `true` when the checkpoint landed, `false` on
/// deadline timeout (timeout path pinned by the paused-clock boundary test
/// below).
pub(crate) async fn poll_storage_until_checkpoint(
    storage: &MemoryStorage,
    session_id: &str,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if storage.load_checkpoint(session_id).await.unwrap().is_some() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Timeout boundary of [`poll_storage_until_checkpoint`]: when the checkpoint
/// never lands, the helper must report `false` at the deadline instead of
/// looping forever. Paused tokio clock (`start_paused = true`): the 100 ms
/// virtual deadline auto-advances instantly, no real wait.
#[tokio::test(start_paused = true)]
async fn test_poll_storage_until_checkpoint_timeout_returns_false() {
    let storage = MemoryStorage::new();
    let landed =
        poll_storage_until_checkpoint(&storage, "never-saved", Duration::from_millis(100)).await;
    assert!(
        !landed,
        "poll must time out (return false) when the checkpoint never lands"
    );
}
