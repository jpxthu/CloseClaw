//! Periodic forgetting cleanup for expired memory events.
//!
//! Scans the `events` table for rows whose `expires_at > 0` and
//! `expires_at <= now`, deletes them and their entity associations,
//! then removes any entities left with zero references.

use rusqlite::Transaction;

use crate::miner::MinerError;

/// Result counters from a single cleanup pass.
#[derive(Debug, Clone, Default)]
pub struct ForgettingCleanupStats {
    /// Number of expired events deleted.
    pub events_deleted: u64,
    /// Number of zero-reference entities deleted.
    pub entities_deleted: u64,
}

/// Run the three-phase forgetting cleanup in a single transaction.
///
/// 1. Delete `event_entities` for expired events.
/// 2. Delete expired events.
/// 3. Delete entities with zero remaining references.
///
/// `expires_at = 0` means the event never expires and is always
/// preserved.
pub(crate) fn cleanup_expired(
    conn: &mut rusqlite::Connection,
    now: i64,
) -> Result<ForgettingCleanupStats, MinerError> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;

    let events_deleted = expire_events(&tx, now)?;
    let entities_deleted = cleanup_orphan_entities(&tx)?;

    tx.commit().map_err(|e| MinerError::Sqlite(e.to_string()))?;

    let stats = ForgettingCleanupStats {
        events_deleted,
        entities_deleted,
    };

    Ok(stats)
}

/// Phase 1+2: delete event_entities and events where expires_at has passed.
fn expire_events(tx: &Transaction<'_>, now: i64) -> Result<u64, MinerError> {
    // Delete associations first (foreign key respect).
    tx.execute(
        "DELETE FROM event_entities WHERE event_id IN \
         (SELECT id FROM events WHERE expires_at > 0 AND expires_at <= ?1)",
        rusqlite::params![now],
    )
    .map_err(|e| MinerError::Sqlite(e.to_string()))?;

    // Delete the expired events themselves.
    let events_deleted = tx
        .execute(
            "DELETE FROM events WHERE expires_at > 0 AND expires_at <= ?1",
            rusqlite::params![now],
        )
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;

    Ok(events_deleted as u64)
}

/// Phase 3: delete entities that have zero references in event_entities.
fn cleanup_orphan_entities(tx: &Transaction<'_>) -> Result<u64, MinerError> {
    let entities_deleted = tx
        .execute(
            "DELETE FROM entities WHERE id NOT IN (SELECT entity_id FROM event_entities)",
            [],
        )
        .map_err(|e| MinerError::Sqlite(e.to_string()))?;

    Ok(entities_deleted as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::miner::{init_schema, MiningEventCategory};
    use crate::test_helpers::{insert_entity_with_event, insert_event};

    #[test]
    fn test_cleanup_expired_removes_expired_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();

        let now = 1000i64;
        let eid = insert_event(&conn, "s1", "a1", MiningEventCategory::Error, now - 1);
        insert_entity_with_event(&conn, "a1", eid, "foo");

        let stats = cleanup_expired(&mut conn, now).unwrap();
        assert_eq!(stats.events_deleted, 1);
        assert_eq!(stats.entities_deleted, 1);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_cleanup_expired_keeps_active_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();

        let now = 1000i64;
        let eid = insert_event(&conn, "s1", "a1", MiningEventCategory::Decision, now + 100);
        insert_entity_with_event(&conn, "a1", eid, "bar");

        let stats = cleanup_expired(&mut conn, now).unwrap();
        assert_eq!(stats.events_deleted, 0);
        assert_eq!(stats.entities_deleted, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_cleanup_expired_keeps_zero_ttl_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();

        let eid = insert_event(&conn, "s1", "a1", MiningEventCategory::Anger, 0);
        insert_entity_with_event(&conn, "a1", eid, "baz");

        let stats = cleanup_expired(&mut conn, 1_000_000).unwrap();
        assert_eq!(stats.events_deleted, 0);
        assert_eq!(stats.entities_deleted, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_cascade_entity_shared_by_surviving_event() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();

        let now = 1000i64;
        let expired_eid = insert_event(&conn, "s1", "a1", MiningEventCategory::Error, now - 1);
        let alive_eid = insert_event(&conn, "s2", "a1", MiningEventCategory::Decision, now + 100);
        // Both events reference the same entity "shared".
        insert_entity_with_event(&conn, "a1", expired_eid, "shared");
        insert_entity_with_event(&conn, "a1", alive_eid, "shared");

        let stats = cleanup_expired(&mut conn, now).unwrap();
        assert_eq!(stats.events_deleted, 1);
        assert_eq!(stats.entities_deleted, 0);

        let entity_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
            .unwrap();
        assert_eq!(entity_count, 1);
    }

    #[test]
    fn test_idempotent_cleanup() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();

        let now = 1000i64;
        let eid = insert_event(&conn, "s1", "a1", MiningEventCategory::Error, now - 1);
        insert_entity_with_event(&conn, "a1", eid, "idem");

        let stats1 = cleanup_expired(&mut conn, now).unwrap();
        assert_eq!(stats1.events_deleted, 1);
        assert_eq!(stats1.entities_deleted, 1);

        let stats2 = cleanup_expired(&mut conn, now).unwrap();
        assert_eq!(stats2.events_deleted, 0);
        assert_eq!(stats2.entities_deleted, 0);
    }

    #[test]
    fn test_empty_db_cleanup() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut conn = rusqlite::Connection::open(tmp.path().join("test.db")).unwrap();
        init_schema(&conn).unwrap();

        let stats = cleanup_expired(&mut conn, 1000).unwrap();
        assert_eq!(stats.events_deleted, 0);
        assert_eq!(stats.entities_deleted, 0);
    }
}
