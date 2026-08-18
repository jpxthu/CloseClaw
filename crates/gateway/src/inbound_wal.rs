//! WAL (Write-Ahead Log) storage for inbound queue persistence.
//!
//! Provides append-only JSONL-based durability for inbound messages.
//! Each entry records a message's arrival; on completion the entry is
//! removed.  On startup, unfinished entries are replayed.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use thiserror::Error;

/// Errors that can occur during WAL operations.
#[derive(Debug, Error)]
pub enum InboundWalError {
    #[error("WAL I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("WAL JSON parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Status of a WAL entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum InboundWalEntryStatus {
    /// Message enqueued but not yet fully processed.
    Pending,
    /// Message fully processed and ready for removal.
    Done,
}

/// A single WAL record representing an enqueued inbound message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundWalEntry {
    /// Unique trace ID for deduplication and correlation.
    pub trace_id: String,
    /// IM platform identifier (e.g. "feishu").
    pub platform: String,
    /// Raw webhook payload, base64-encoded to keep JSONL safe.
    pub raw_payload: String,
    /// Peer / chat ID for routing.
    pub peer_id: String,
    /// Unix timestamp (seconds) when the entry was created.
    pub enqueued_at: i64,
    /// Current status of this entry.
    pub status: InboundWalEntryStatus,
}

impl InboundWalEntry {
    /// Create a new pending entry from raw bytes.
    pub fn new(trace_id: String, platform: String, raw_payload: &[u8], peer_id: String) -> Self {
        Self {
            trace_id,
            platform,
            raw_payload: BASE64.encode(raw_payload),
            peer_id,
            enqueued_at: chrono::Utc::now().timestamp(),
            status: InboundWalEntryStatus::Pending,
        }
    }

    /// Decode the base64 raw payload back to bytes.
    pub fn decoded_payload(&self) -> Result<Vec<u8>, base64::DecodeError> {
        BASE64.decode(&self.raw_payload)
    }
}

/// Handle to a WAL directory containing a single `inbound.jsonl` file.
///
/// All public methods are safe to call from async contexts because
/// blocking I/O is performed under a `Mutex` that never crosses await
/// points.
pub struct InboundWal {
    #[allow(dead_code)] // read by `dir()` which is cfg(test)-only
    dir: PathBuf,
    file_path: PathBuf,
    lock: Mutex<()>,
}

impl InboundWal {
    /// Open (or create) a WAL in the given directory.
    ///
    /// Creates the directory and `inbound.jsonl` if they do not exist.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, InboundWalError> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let file_path = dir.join("inbound.jsonl");
        // Touch the file so it exists for later appends.
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)?;
        Ok(Self {
            dir,
            file_path,
            lock: Mutex::new(()),
        })
    }

    /// Append a single entry to the WAL and fsync.
    pub fn append(&self, entry: &InboundWalEntry) -> Result<(), InboundWalError> {
        let _guard = self.lock.lock().expect("WAL lock poisoned");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)?;
        let mut line = serde_json::to_vec(entry)?;
        line.push(b'\n');
        file.write_all(&line)?;
        file.sync_all()?;
        Ok(())
    }

    /// Load all entries from the WAL file in line order.
    ///
    /// Malformed lines are skipped with a warning; the WAL is never
    /// truncated by this method.
    pub fn load_all(&self) -> Result<Vec<InboundWalEntry>, InboundWalError> {
        let _guard = self.lock.lock().expect("WAL lock poisoned");
        let mut entries = Vec::new();
        let file = match File::open(&self.file_path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
            Err(e) => return Err(e.into()),
        };
        let reader = BufReader::new(file);
        for (idx, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(line = idx + 1, error = %e, "WAL: failed to read line");
                    continue;
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<InboundWalEntry>(trimmed) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    tracing::warn!(
                        line = idx + 1,
                        error = %e,
                        "WAL: skipping malformed line"
                    );
                }
            }
        }
        Ok(entries)
    }

    /// Mark all entries with the given `trace_id` as Done and remove
    /// them from the file.
    ///
    /// The implementation rewrites the file omitting every matching
    /// entry.  A full rewrite is acceptable because:
    /// - Entries are removed promptly after processing.
    /// - The file is small (bounded by queue capacity).
    pub fn mark_done_and_delete(&self, trace_id: &str) -> Result<(), InboundWalError> {
        let _guard = self.lock.lock().expect("WAL lock poisoned");
        let entries = self.read_entries_unlocked()?;
        let remaining: Vec<&InboundWalEntry> =
            entries.iter().filter(|e| e.trace_id != trace_id).collect();
        self.write_entries_unlocked(&remaining)?;
        Ok(())
    }

    /// Return the directory this WAL resides in.
    #[cfg(test)]
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }
}

// ── internal helpers (called while lock is held) ──────────────

impl InboundWal {
    fn read_entries_unlocked(&self) -> Result<Vec<InboundWalEntry>, InboundWalError> {
        let file = match File::open(&self.file_path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for (idx, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<InboundWalEntry>(trimmed) {
                entries.push(entry);
            } else {
                tracing::warn!(
                    line = idx + 1,
                    "WAL: skipping malformed line during rewrite"
                );
            }
        }
        Ok(entries)
    }

    fn write_entries_unlocked(&self, entries: &[&InboundWalEntry]) -> Result<(), InboundWalError> {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.file_path)?;
        for entry in entries {
            let mut line = serde_json::to_vec(entry)?;
            line.push(b'\n');
            file.write_all(&line)?;
        }
        file.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_wal() -> (tempfile::TempDir, InboundWal) {
        let dir = tempfile::tempdir().unwrap();
        let wal = InboundWal::open(dir.path()).unwrap();
        (dir, wal)
    }

    fn sample_entry(trace_id: &str) -> InboundWalEntry {
        InboundWalEntry::new(
            trace_id.to_string(),
            "feishu".to_string(),
            b"{\"event\":{}}",
            "p1".to_string(),
        )
    }

    #[test]
    fn open_creates_directory_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("nested").join("wal");
        let wal = InboundWal::open(&sub).unwrap();
        assert!(sub.exists());
        assert!(sub.join("inbound.jsonl").exists());
        assert_eq!(wal.dir(), sub.as_path());
    }

    #[test]
    fn append_and_load_roundtrip() {
        let (_dir, wal) = temp_wal();
        let e1 = sample_entry("tr-1");
        let e2 = sample_entry("tr-2");
        wal.append(&e1).unwrap();
        wal.append(&e2).unwrap();
        let loaded = wal.load_all().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].trace_id, "tr-1");
        assert_eq!(loaded[1].trace_id, "tr-2");
    }

    #[test]
    fn mark_done_and_delete_removes_entry() {
        let (_dir, wal) = temp_wal();
        wal.append(&sample_entry("tr-1")).unwrap();
        wal.append(&sample_entry("tr-2")).unwrap();
        wal.mark_done_and_delete("tr-1").unwrap();
        let loaded = wal.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].trace_id, "tr-2");
    }

    #[test]
    fn mark_done_and_delete_nonexistent_is_noop() {
        let (_dir, wal) = temp_wal();
        wal.append(&sample_entry("tr-1")).unwrap();
        wal.mark_done_and_delete("no-such-id").unwrap();
        let loaded = wal.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn load_all_empty_file() {
        let (_dir, wal) = temp_wal();
        let loaded = wal.load_all().unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn load_all_skips_malformed_lines() {
        let (_dir, wal) = temp_wal();
        wal.append(&sample_entry("tr-1")).unwrap();
        // Append a raw bad line directly.
        use std::io::Write;
        let mut f = OpenOptions::new()
            .append(true)
            .open(&wal.file_path)
            .unwrap();
        writeln!(f, "{{not valid json}}").unwrap();
        wal.append(&sample_entry("tr-2")).unwrap();
        let loaded = wal.load_all().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].trace_id, "tr-1");
        assert_eq!(loaded[1].trace_id, "tr-2");
    }

    #[test]
    fn payload_base64_roundtrip() {
        let payload = b"\xff\xfe binary \x00 data";
        let entry = InboundWalEntry::new("tr-bin".into(), "test".into(), payload, "peer".into());
        let decoded = entry.decoded_payload().unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn load_all_handles_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let wal = InboundWal::open(dir.path()).unwrap();
        // Remove the file to simulate a crash before first write.
        fs::remove_file(wal.file_path.clone()).unwrap();
        let loaded = wal.load_all().unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn append_after_file_removed_recreates() {
        let (_dir, wal) = temp_wal();
        wal.append(&sample_entry("tr-1")).unwrap();
        // Simulate unexpected deletion.
        fs::remove_file(&wal.file_path).unwrap();
        wal.append(&sample_entry("tr-2")).unwrap();
        let loaded = wal.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].trace_id, "tr-2");
    }

    #[test]
    fn entry_status_serialization() {
        let pending = InboundWalEntryStatus::Pending;
        let json = serde_json::to_string(&pending).unwrap();
        let back: InboundWalEntryStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, InboundWalEntryStatus::Pending);
    }
}
