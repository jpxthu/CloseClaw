//! Integration test helpers for closeclaw
//!
//! Provides reusable shared test setup utilities for integration tests.
//! All functions are inline (no dependency on `pub(crate)` functions from `src/`).

#[cfg(feature = "fake-llm")]
use std::path::PathBuf;
#[cfg(feature = "fake-llm")]
use std::sync::Arc;

#[cfg(feature = "fake-llm")]
use closeclaw::llm::fake::FakeProvider;
#[cfg(feature = "fake-llm")]
use closeclaw::llm::provider::Provider;
#[cfg(feature = "fake-llm")]
use closeclaw::llm::session::ConversationSession;
#[cfg(feature = "fake-llm")]
use closeclaw::llm::ChatSession;
#[cfg(feature = "fake-llm")]
use closeclaw::llm::LLMRegistry;
#[cfg(feature = "fake-llm")]
use closeclaw::session::storage::sqlite::SqliteStorage;
#[cfg(feature = "fake-llm")]
use closeclaw::session::PendingMessage;

/// Creates a new temporary test root directory.
///
/// The returned `TempDir` lives at `/tmp/closeclaw-test-<uuid>/` and is
/// automatically cleaned up when dropped.
///
/// # Returns
/// A `tempfile::TempDir` wrapping the temporary directory path.
#[cfg(feature = "fake-llm")]
pub fn new_test_root() -> tempfile::TempDir {
    tempfile::TempDir::with_prefix("closeclaw-test-")
        .expect("new_test_root: failed to create temp dir")
}

/// Sets up a `ConversationSession` backed by a temporary `SqliteStorage`,
/// with a `FakeProvider` registered in an `LLMRegistry`.
///
/// This function initializes all components together. The returned `FakeProvider` is registered to
/// `LLMRegistry`（以 `"fake"` 为 key），可直接通过 `session.push_pending`
/// 等 inherent 方法使用。
///
/// # Returns
/// A tuple of `(session, storage, fake_provider, test_root)`:
/// - `session`: A `ConversationSession` with `FakeProvider` model configured
/// - `storage`: A `SqliteStorage` with the temporary data dir
/// - `fake_provider`: A `FakeProvider` (registered in `registry` with key `"fake"`)
/// - `test_root`: The `TempDir` that backs `storage`
#[cfg(feature = "fake-llm")]
pub fn setup_session_with_storage() -> (
    ConversationSession,
    SqliteStorage,
    FakeProvider,
    tempfile::TempDir,
) {
    let test_root = new_test_root();
    let data_path = test_root.path().to_path_buf();

    let storage = SqliteStorage::new(&data_path)
        .expect("setup_session_with_storage: SqliteStorage::new failed");

    let registry = Arc::new(LLMRegistry::new());
    let fake_provider = FakeProvider::builder()
        .then_ok("fake response", "fake-model")
        .build();
    let wrapped: Arc<dyn Provider> = Arc::new(fake_provider.clone());
    let runtime = tokio::runtime::Runtime::new()
        .expect("setup_session_with_storage: failed to create tokio runtime");
    runtime.block_on(async {
        registry.register("fake".to_string(), wrapped).await;
    });

    let session = ConversationSession::new(
        "test-session".to_string(),
        "fake-model".to_string(),
        PathBuf::from("/tmp"),
    );

    (session, storage, fake_provider, test_root)
}

// ---------------------------------------------------------------------------
// Tests (only compiled when fake-llm feature is enabled)
// ---------------------------------------------------------------------------

#[cfg(feature = "fake-llm")]
#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // new_test_root tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_new_test_root_path_prefix() {
        let root = new_test_root();
        let path = root.path();
        assert!(
            path.to_str()
                .unwrap_or("")
                .starts_with("/tmp/closeclaw-test-"),
            "path should start with /tmp/closeclaw-test-, got {}",
            path.display()
        );
    }

    #[test]
    fn test_new_test_root_directory_exists() {
        let root = new_test_root();
        assert!(
            root.path().exists(),
            "temp dir should exist, got {}",
            root.path().display()
        );
    }

    #[test]
    fn test_new_test_root_unique_per_call() {
        let root1 = new_test_root();
        let root2 = new_test_root();
        assert_ne!(
            root1.path(),
            root2.path(),
            "two calls should return different paths"
        );
    }

    #[test]
    fn test_new_test_root_cleanup_on_drop() {
        let path = {
            let root = new_test_root();
            root.path().to_path_buf()
        };
        // TempDir is dropped, path should not exist
        assert!(
            !path.exists(),
            "temp dir should be cleaned up after drop, but {} still exists",
            path.display()
        );
    }

    // ---------------------------------------------------------------------------
    // setup_session_with_storage tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_setup_session_with_storage_returns_non_null_session() {
        let (session, _storage, _fake, _root) = setup_session_with_storage();
        // Verify session has the expected session_id
        assert_eq!(
            session.turn_count(),
            0,
            "new session should have turn_count 0"
        );
    }

    #[test]
    fn test_setup_session_with_storage_returns_non_null_storage() {
        let (_session, storage, _fake, _root) = setup_session_with_storage();
        // SqliteStorage is created in the temp dir — no easy direct assertion
        // beyond "didn't panic". Verify storage data dir exists.
        let data_dir = storage.data_dir();
        assert!(
            data_dir.exists(),
            "storage data_dir should exist at {}",
            data_dir.display()
        );
    }

    #[test]
    fn test_setup_session_with_storage_returns_working_fake_provider() {
        let (mut session, _storage, fake_provider, _root) = setup_session_with_storage();

        // push_pending / pop_pending are inherent methods on ConversationSession
        let msg = PendingMessage::new("test-msg-1".to_string(), "hello".to_string());
        session.push_pending(msg);
        let popped = session.pop_pending();
        assert!(
            popped.is_some(),
            "pop_pending should return the pushed message"
        );
        assert_eq!(
            popped.as_ref().unwrap().message_id,
            "test-msg-1",
            "popped message should have correct message_id"
        );

        // set_llm_busy is an inherent method
        session.set_llm_busy(true);
        assert!(
            session.is_llm_busy(),
            "is_llm_busy should reflect the busy state"
        );
        session.set_llm_busy(false);
        assert!(
            !session.is_llm_busy(),
            "is_llm_busy should be false after set_llm_busy(false)"
        );

        // Verify fake_provider is a valid FakeProvider by checking captured requests
        // (empty since we haven't sent any chat requests, but that's fine)
        let captured = fake_provider.captured_requests();
        assert!(
            captured.is_empty(),
            "fresh FakeProvider should have no captured requests, got {}",
            captured.len()
        );
    }

    #[test]
    fn test_setup_concurrent_sessions_do_not_interfere() {
        let (mut session1, _storage1, _fake1, _root1) = setup_session_with_storage();
        let (mut session2, _storage2, _fake2, _root2) = setup_session_with_storage();

        let msg1 = PendingMessage::new("concurrent-1".to_string(), "hello".to_string());
        let msg2 = PendingMessage::new("concurrent-2".to_string(), "world".to_string());

        session1.push_pending(msg1);
        session2.push_pending(msg2);

        // Each session should only see its own message
        let popped1 = session1.pop_pending().map(|m| m.message_id);
        let popped2 = session2.pop_pending().map(|m| m.message_id);
        assert_eq!(popped1.as_deref(), Some("concurrent-1"));
        assert_eq!(popped2.as_deref(), Some("concurrent-2"));
    }
}
