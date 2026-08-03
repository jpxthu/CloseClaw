//! Per-file mutex map for serializing concurrent writes to the same file.
//!
//! [`FileMutexMap`] assigns an async [`tokio::sync::Mutex`] to each canonical
//! file path.  Concurrent tool calls targeting the **same** file are serialized;
//! calls targeting **different** files proceed in parallel.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{Mutex, OwnedMutexGuard};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Non-blocking acquire result.
pub enum TryAcquireResult {
    /// Lock acquired — the owned guard is returned.
    Acquired(OwnedMutexGuard<()>),
    /// The lock is currently held by another task.
    WouldBlock,
}

/// Per-file mutex map.
///
/// Keys are **canonicalized** paths (symlinks resolved).  Each entry holds an
/// [`Arc<Mutex<()>>`] that is lazily removed from the map when no external
/// references remain.
pub struct FileMutexMap {
    inner: DashMap<PathBuf, Arc<Mutex<()>>>,
}

impl Default for FileMutexMap {
    fn default() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }
}

impl FileMutexMap {
    /// Create an empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the mutex for `path`, blocking until available.
    ///
    /// The returned [`OwnedMutexGuard`] automatically releases the lock on drop.
    /// A cloned [`Arc`] is returned alongside to ensure the inner mutex stays
    /// alive while the guard is live.
    ///
    /// # Panics
    ///
    /// Panics if `path` cannot be canonicalized.
    pub async fn acquire(&self, path: &Path) -> (OwnedMutexGuard<()>, Arc<Mutex<()>>) {
        let canonical = canonicalize_or_clone(path);
        let mutex = self
            .inner
            .entry(canonical)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let guard = Arc::clone(&mutex).lock_owned().await;
        (guard, mutex)
    }

    /// Non-blocking attempt to acquire the mutex for `path`.
    ///
    /// Uses [`tokio::sync::Mutex::try_lock_owned`] which consumes an [`Arc`]
    /// clone and returns an [`OwnedMutexGuard`].  The DashMap entry retains its
    /// own `Arc`, keeping the inner mutex alive.
    ///
    /// Returns [`TryAcquireResult::WouldBlock`] if the lock is currently held.
    pub fn try_acquire(&self, path: &Path) -> TryAcquireResult {
        let canonical = canonicalize_or_clone(path);
        let mutex = self
            .inner
            .entry(canonical)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        // `try_lock_owned` consumes `self: Arc<Mutex<T>>` and returns an
        // owned guard.  The DashMap retains its own Arc clone, so the mutex
        // stays alive even after this clone is consumed.
        match mutex.clone().try_lock_owned() {
            Ok(guard) => TryAcquireResult::Acquired(guard),
            Err(_) => TryAcquireResult::WouldBlock,
        }
    }

    /// Remove the entry for `path` if no external references remain.
    ///
    /// This is called opportunistically — not on every drop — to keep the map
    /// bounded without adding per-guard bookkeeping.
    pub fn cleanup(&self, path: &Path) {
        let canonical = canonicalize_or_clone(path);
        if let Some(entry) = self.inner.get(&canonical) {
            // strong_count == 1 means only the DashMap holds a reference.
            if Arc::strong_count(&entry) == 1 {
                drop(entry);
                self.inner.remove(&canonical);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Canonicalize `path` if possible; fall back to the original path.
fn canonicalize_or_clone(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "file_mutex_tests.rs"]
mod tests;
