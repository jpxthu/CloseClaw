//! Audit logger — buffered async writer to JSONL files

use chrono::Local;
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;
use tracing::{error, info};

use super::{AuditEvent, AuditResult};

/// Audit logger — writes events to ~/.closeclaw/audit/YYYY-MM-DD.jsonl
pub struct AuditLogger {
    /// Base directory for audit logs
    base_dir: PathBuf,
    /// In-memory buffer of events
    buffer: Arc<Mutex<VecDeque<AuditEvent>>>,
    /// Flush interval in seconds (currently unused but reserved for future timer-based flush)
    #[allow(dead_code)]
    flush_interval_secs: u64,
    /// Max buffer size before forced flush
    max_buffer_size: usize,
    /// Current file date (to detect day changes); uses std Mutex for sync access from log_file_path
    #[allow(dead_code)]
    pub(crate) current_date: StdMutex<String>,
}

impl AuditLogger {
    /// Create a new AuditLogger with the default audit directory
    pub fn new() -> Self {
        let home = std::env::var("HOME").expect("HOME not set");
        let base_dir = PathBuf::from(home).join(".closeclaw").join("audit");

        Self {
            base_dir,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            flush_interval_secs: 5,
            max_buffer_size: 500,
            current_date: StdMutex::new(String::new()),
        }
    }

    /// Create with custom base directory (useful for testing)
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            flush_interval_secs: 5,
            max_buffer_size: 500,
            current_date: StdMutex::new(String::new()),
        }
    }

    /// Get the current date string (YYYY-MM-DD)
    pub(crate) fn current_date_string() -> String {
        Local::now().format("%Y-%m-%d").to_string()
    }

    /// Get the log file path for the current (possibly mocked) date
    fn log_file_path(&self) -> PathBuf {
        // Use the locked current_date if set, otherwise fall back to today's real date
        let date = {
            let guard = self.current_date.lock().unwrap();
            if guard.is_empty() {
                drop(guard);
                Self::current_date_string()
            } else {
                guard.clone()
            }
        };
        self.base_dir.join(format!("{}.jsonl", date))
    }

    /// Ensure the audit directory exists
    fn ensure_dir(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.base_dir)
    }

    /// Write a single event to the file synchronously
    fn write_event_to_file(path: &PathBuf, event: &AuditEvent) -> std::io::Result<()> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;

        let mut writer = BufWriter::new(file);
        writer.write_all(event.serialize_to_json().as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    /// Log an audit event (buffers and periodically flushes)
    pub async fn log(&self, event: AuditEvent) {
        // Always emit to tracing
        let event_type_str = format!("{:?}", event.event_type);
        match event.result {
            AuditResult::Allow => {
                info!(event_type = %event_type_str, "audit: {:?}", serde_json::to_string(&event.details).unwrap_or_default())
            }
            AuditResult::Deny => {
                info!(event_type = %event_type_str, result = "deny", "audit: {:?}", serde_json::to_string(&event.details).unwrap_or_default())
            }
            AuditResult::Error => {
                error!(event_type = %event_type_str, "audit error: {:?}", serde_json::to_string(&event.details).unwrap_or_default())
            }
        }

        // Buffer the event
        {
            let mut buf = self.buffer.lock().await;
            buf.push_back(event.clone());
        }

        // Check if we should flush
        let should_flush = {
            let buf = self.buffer.lock().await;
            buf.len() >= self.max_buffer_size
        };

        if should_flush {
            self.flush().await;
        }
    }

    /// Flush buffered events to disk
    pub async fn flush(&self) {
        let events: Vec<AuditEvent> = {
            let mut buf = self.buffer.lock().await;
            buf.drain(..).collect()
        };

        if events.is_empty() {
            return;
        }

        if let Err(e) = self.ensure_dir() {
            error!("failed to create audit directory: {}", e);
            return;
        }

        let path = self.log_file_path();
        for event in &events {
            if let Err(e) = Self::write_event_to_file(&path, event) {
                error!("failed to write audit event to {}: {}", path.display(), e);
            }
        }

        info!(path = %path.display(), count = events.len(), "audit log flushed");
    }

    /// Flush on drop if there are buffered events
    pub async fn shutdown(&self) {
        self.flush().await;
    }

    /// Rotate if needed (called periodically; today is a new day)
    pub async fn rotate_if_needed(&self) {
        let today = Self::current_date_string();
        let needs_flush = {
            let mut current = self.current_date.lock().unwrap();
            if *current != today {
                *current = today;
                true
            } else {
                false
            }
        };
        if needs_flush {
            self.flush().await;
        }
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new()
    }
}
