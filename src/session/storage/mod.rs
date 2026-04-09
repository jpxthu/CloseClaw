//! Storage backends for session persistence
//!
//! This module provides pluggable storage backends for the persistence layer.
//!
//! - [`MemoryStorage`] — In-memory storage (for testing)
//! - [`RedisStorage`] — Redis backend (requires redis dependency)

pub mod memory;
pub mod redis;

// Re-export the storage types
pub use memory::MemoryStorage;
pub use redis::RedisStorage;
