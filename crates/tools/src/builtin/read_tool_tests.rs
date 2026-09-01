//! ReadTool integration tests — offset/limit, truncation, dedup cache.

use crate::builtin::file_ops::ReadTool;
use crate::Tool;
use crate::ToolContext;
use closeclaw_common::tool_session::ToolSession;
use closeclaw_common::{FileReadCache, ReadRange};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;
use tempfile::TempDir;

use super::file_ops::tests::{
    allow_file, allow_tool, make_af, make_cm, make_ctx, make_engine, make_sm,
};

// ---------------------------------------------------------------------------
// Mock ToolSession for dedup cache tests
// ---------------------------------------------------------------------------

/// Mock ToolSession that supports file-read dedup cache.
pub(crate) struct MockReadSession {
    file_read_cache: Mutex<HashMap<String, FileReadCache>>,
    file_mtimes: Mutex<HashMap<String, Option<SystemTime>>>,
}

impl MockReadSession {
    pub(crate) fn new() -> Self {
        Self {
            file_read_cache: Mutex::new(HashMap::new()),
            file_mtimes: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn insert_cache(&self, path: &str, cache: FileReadCache) {
        self.file_read_cache
            .lock()
            .unwrap()
            .insert(path.to_string(), cache);
    }
}

#[async_trait::async_trait]
impl ToolSession for MockReadSession {
    async fn register_tool_handle(
        &self,
        _call_id: String,
        _handle: std::sync::Arc<dyn closeclaw_common::tool_session::KillHandle>,
    ) {
    }

    async fn record_file_read(&self, path: &str, mtime: Option<SystemTime>) {
        self.file_mtimes
            .lock()
            .unwrap()
            .insert(path.to_string(), mtime);
    }

    fn get_file_mtime(&self, path: &str) -> Option<SystemTime> {
        self.file_mtimes
            .lock()
            .unwrap()
            .get(path)
            .copied()
            .flatten()
    }

    fn get_file_read_cache(&self, path: &str) -> Option<FileReadCache> {
        self.file_read_cache.lock().unwrap().get(path).cloned()
    }

    async fn record_file_read_range(
        &self,
        path: &str,
        mtime: Option<SystemTime>,
        range: ReadRange,
    ) {
        let mut cache = self.file_read_cache.lock().unwrap();
        let entry = cache
            .entry(path.to_string())
            .or_insert_with(|| FileReadCache {
                mtime,
                ranges: Vec::new(),
            });
        entry.ranges.push(range);
    }
}

fn make_ctx_with_session(session: std::sync::Arc<dyn ToolSession>) -> ToolContext {
    ToolContext {
        agent_id: "a".to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: Some(session),
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

// ---------------------------------------------------------------------------
// ReadTool integration tests — offset/limit, truncation, dedup
// ---------------------------------------------------------------------------

/// offset/limit parameters are parsed and applied correctly.
#[tokio::test]
async fn test_read_offset_limit_parsing() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("lines.txt");
    let content: String = (1..=20).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file, &content).unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "read"),
    ];
    let tool = ReadTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": file.to_str().unwrap(),
        "offset": 5,
        "limit": 3
    });
    let result = tool.call(args, &make_ctx("a")).await.unwrap();
    let text = result.data["content"].as_str().unwrap();
    assert!(text.starts_with("line 5"));
    assert!(text.contains("line 6"));
    assert!(text.contains("line 7"));
    assert!(!text.contains("line 8"));
}

/// Large file triggers truncation with continuation hint in output.
#[tokio::test]
async fn test_read_large_file_truncation_with_hint() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("big.txt");
    let content: String = (1..=2500).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file, &content).unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "read"),
    ];
    let tool = ReadTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({ "path": file.to_str().unwrap() });
    let result = tool.call(args, &make_ctx("a")).await.unwrap();
    let text = result.data["content"].as_str().unwrap();
    assert!(text.starts_with("line 1"));
    assert!(text.contains("[Showing lines 1-2000 of 2500. Use offset=2001 to continue.]"));
}

/// Dedup cache hit returns "File unchanged since last read."
#[tokio::test]
async fn test_read_dedup_cache_hit() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("cached.txt");
    std::fs::write(&file, "content").unwrap();
    let mtime = std::fs::metadata(&file).unwrap().modified().ok();

    let session = std::sync::Arc::new(MockReadSession::new());
    session.insert_cache(
        &file.to_string_lossy(),
        FileReadCache {
            mtime,
            ranges: vec![ReadRange {
                offset: 1,
                limit: None,
            }],
        },
    );

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "read"),
    ];
    let tool = ReadTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({ "path": file.to_str().unwrap() });
    let result = tool
        .call(args, &make_ctx_with_session(session))
        .await
        .unwrap();
    assert_eq!(result.data["content"], "File unchanged since last read.");
}

/// Dedup cache miss (different range) → normal read.
#[tokio::test]
async fn test_read_dedup_cache_miss_different_range() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("range_miss.txt");
    let content: String = (1..=10).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file, &content).unwrap();
    let mtime = std::fs::metadata(&file).unwrap().modified().ok();

    let session = std::sync::Arc::new(MockReadSession::new());
    session.insert_cache(
        &file.to_string_lossy(),
        FileReadCache {
            mtime,
            ranges: vec![ReadRange {
                offset: 1,
                limit: None,
            }],
        },
    );

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "read"),
    ];
    let tool = ReadTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": file.to_str().unwrap(),
        "offset": 3,
        "limit": 2
    });
    let result = tool
        .call(args, &make_ctx_with_session(session))
        .await
        .unwrap();
    let text = result.data["content"].as_str().unwrap();
    assert!(text.starts_with("line 3"));
    assert!(text.contains("line 4"));
    assert!(!text.contains("line 5"));
}

/// Dedup cache miss (mtime changed) → normal read.
#[tokio::test]
async fn test_read_dedup_cache_miss_mtime_changed() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("mtime_miss.txt");
    std::fs::write(&file, "original").unwrap();
    let mtime = std::fs::metadata(&file).unwrap().modified().ok();

    let session = std::sync::Arc::new(MockReadSession::new());
    let fake_mtime = mtime.map(|t| t + std::time::Duration::from_secs(3600));
    session.insert_cache(
        &file.to_string_lossy(),
        FileReadCache {
            mtime: fake_mtime,
            ranges: vec![ReadRange {
                offset: 1,
                limit: None,
            }],
        },
    );

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "read"),
    ];
    let tool = ReadTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({ "path": file.to_str().unwrap() });
    let result = tool
        .call(args, &make_ctx_with_session(session))
        .await
        .unwrap();
    let text = result.data["content"].as_str().unwrap();
    assert_ne!(text, "File unchanged since last read.");
    assert!(text.contains("original"));
}

/// Large file with offset continues from correct position.
#[tokio::test]
async fn test_read_large_file_with_offset_continuation() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("continuation.txt");
    let content: String = (1..=2500).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file, &content).unwrap();

    let rules = vec![
        allow_tool("a", "file_ops"),
        allow_file("a", "/tmp/**", "read"),
    ];
    let tool = ReadTool::new(make_engine(rules), make_sm(), make_cm(), make_af());
    let args = serde_json::json!({
        "path": file.to_str().unwrap(),
        "offset": 2001
    });
    let result = tool.call(args, &make_ctx("a")).await.unwrap();
    let text = result.data["content"].as_str().unwrap();
    assert!(text.starts_with("line 2001"));
    assert!(text.contains("line 2500"));
    assert!(!text.contains("Use offset="));
}
