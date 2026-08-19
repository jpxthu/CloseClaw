//! Generic JSON Lines file writer with truncation support.
//!
//! Shared by [`FileAuditLogger`] and [`FileRejectionLogger`] to avoid
//! duplicating the prepend-reverse-chronological-order write logic.

use serde::Serialize;
use std::fs::OpenOptions;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Generic JSON Lines file writer with optional entry limit.
///
/// Entries are stored in reverse chronological order (newest first).
/// When `max_entries` is `Some(n)`, old entries are truncated on write
/// to keep at most `n` lines.
pub struct JsonlFileWriter {
    path: PathBuf,
    max_entries: Option<usize>,
    writer: Mutex<()>,
}

impl JsonlFileWriter {
    /// Create a new writer that appends to the given path.
    /// Parent directories are created if they don't exist.
    pub fn new(path: PathBuf) -> std::io::Result<Self> {
        Self::new_with_limit(path, None)
    }

    /// Create a new writer with a maximum entry limit.
    pub fn new_with_limit(path: PathBuf, max_entries: Option<usize>) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            max_entries,
            writer: Mutex::new(()),
        })
    }

    /// Returns the path this writer targets.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the configured maximum entry limit, if any.
    pub fn max_entries(&self) -> Option<usize> {
        self.max_entries
    }

    /// Count non-empty lines in the log file.
    pub fn count_entries(path: &Path) -> usize {
        std::fs::File::open(path)
            .map(|f| {
                io::BufReader::new(f)
                    .lines()
                    .map_while(Result::ok)
                    .filter(|l| !l.trim().is_empty())
                    .count()
            })
            .unwrap_or(0)
    }

    /// Truncate old entries, keeping the newest `keep` lines.
    fn truncate_old_entries(path: &Path, keep: usize) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() <= keep {
            return;
        }
        let kept: String = lines.iter().take(keep).map(|l| format!("{l}\n")).collect();
        let _ = std::fs::write(path, kept);
    }

    /// Write a single serialized entry, prepending it so the newest
    /// entry is always at the top (reverse chronological order).
    fn write_entry<T: Serialize>(&self, entry: &T) {
        let new_line = match serde_json::to_vec(entry) {
            Ok(mut line) => {
                line.push(b'\n');
                line
            }
            Err(_) => return,
        };
        let existing = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut combined = new_line;
        combined.extend_from_slice(existing.as_bytes());
        let _ = std::fs::write(&self.path, combined);
    }

    /// Write a single entry, enforcing the entry limit.
    pub fn write<T: Serialize>(&self, entry: &T) {
        let _lock = self.writer.lock();

        if let Some(max) = self.max_entries {
            let count = Self::count_entries(&self.path);
            if count >= max {
                Self::truncate_old_entries(&self.path, max - 1);
            }
        }

        self.write_entry(entry);
    }
}

impl std::fmt::Debug for JsonlFileWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonlFileWriter")
            .field("path", &self.path)
            .field("max_entries", &self.max_entries)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestEntry {
        name: String,
        value: i32,
    }

    #[test]
    fn test_count_entries_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.log");
        std::fs::write(&path, "").unwrap();
        assert_eq!(JsonlFileWriter::count_entries(&path), 0);
    }

    #[test]
    fn test_count_entries_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.log");
        std::fs::write(&path, "line1\nline2\nline3\n").unwrap();
        assert_eq!(JsonlFileWriter::count_entries(&path), 3);
    }

    #[test]
    fn test_count_entries_skips_blank_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.log");
        std::fs::write(&path, "line1\n\nline2\n  \nline3\n").unwrap();
        assert_eq!(JsonlFileWriter::count_entries(&path), 3);
    }

    #[test]
    fn test_write_single_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.jsonl");
        let writer = JsonlFileWriter::new(path.clone()).unwrap();

        let entry = TestEntry {
            name: "a".into(),
            value: 1,
        };
        writer.write(&entry);

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: TestEntry = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn test_write_prepends_multiple() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.jsonl");
        let writer = JsonlFileWriter::new(path.clone()).unwrap();

        for i in 0..3 {
            writer.write(&TestEntry {
                name: format!("e{}", i),
                value: i as i32,
            });
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        // Newest first
        for (i, line) in lines.iter().enumerate() {
            let parsed: TestEntry = serde_json::from_str(line).unwrap();
            assert_eq!(parsed.name, format!("e{}", 2 - i));
        }
    }

    #[test]
    fn test_write_with_limit_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.jsonl");
        let writer = JsonlFileWriter::new_with_limit(path.clone(), Some(3)).unwrap();

        for i in 0..5 {
            writer.write(&TestEntry {
                name: format!("e{}", i),
                value: i as i32,
            });
        }

        let count = JsonlFileWriter::count_entries(&path);
        assert_eq!(count, 3);

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        for (i, line) in lines.iter().enumerate() {
            let parsed: TestEntry = serde_json::from_str(line).unwrap();
            assert_eq!(parsed.name, format!("e{}", 4 - i));
        }
    }

    #[test]
    fn test_write_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("dir").join("out.jsonl");
        let writer = JsonlFileWriter::new(path.clone()).unwrap();
        writer.write(&TestEntry {
            name: "a".into(),
            value: 1,
        });
        assert!(path.exists());
    }

    #[test]
    fn test_debug_impl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.jsonl");
        let writer = JsonlFileWriter::new(path).unwrap();
        let debug_str = format!("{:?}", writer);
        assert!(debug_str.contains("JsonlFileWriter"));
    }

    #[test]
    fn test_new_with_limit_no_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.jsonl");
        let writer = JsonlFileWriter::new_with_limit(path, None).unwrap();
        assert_eq!(writer.max_entries(), None);
    }

    #[test]
    fn test_new_with_limit_with_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.jsonl");
        let writer = JsonlFileWriter::new_with_limit(path, Some(5)).unwrap();
        assert_eq!(writer.max_entries(), Some(5));
    }
}
