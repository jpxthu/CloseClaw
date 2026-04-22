//! Storage backends for session persistence
//!
//! This module provides pluggable storage backends for the persistence layer.
//!
//! - [`MemoryStorage`] — In-memory storage (for testing)
//! - [`RedisStorage`] — Redis backend (requires redis dependency)
//! - [`SqliteStorage`] — SQLite backend (SQLite metadata table + JSONL transcript files; archive/restore/purge; two-phase transaction for atomicity)

pub mod memory;
pub mod redis;
pub mod sqlite;

// Re-export the storage types
pub use memory::MemoryStorage;
pub use redis::RedisStorage;
pub use sqlite::SqliteStorage;

#[cfg(test)]
mod sqlite_tests;
