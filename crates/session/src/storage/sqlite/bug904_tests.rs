//! Bug #904 tests for SQLite storage

use crate::storage::SqliteStorage;

// ===================================================================
// 10. Bug #904: init_schema idempotent with existing thread_id column
// ===================================================================
#[tokio::test]
async fn test_init_schema_idempotent_with_existing_thread_id_column() {
    let temp = tempfile::TempDir::new().unwrap();
    let db_path = temp.path().join("test.db");

    // Create a DB and manually add thread_id column (simulating a DB from a previous version)
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                role TEXT NOT NULL,
                channel TEXT NOT NULL,
                chat_id TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                title TEXT,
                last_message_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                archived_at INTEGER,
                message_count INTEGER DEFAULT 0,
                metadata TEXT,
                thread_id TEXT
            );
            "#,
        )
        .unwrap();
    }

    // Now open via SqliteStorage::new which calls init_schema — should not error
    let result = SqliteStorage::new(temp.path());
    assert!(
        result.is_ok(),
        "init_schema should be idempotent: {:?}",
        result.err()
    );
}
