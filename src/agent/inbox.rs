//! Inbox Module - Multi-Agent Communication with Persistence and Retry
//!
//! Implements reliable message delivery between agents with:
//! - Exponential backoff retry
//! - Dead letter handling
//! - Jitter to prevent thundering herd
//! - Stats and monitoring

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;
use tokio::sync::RwLock;
use uuid::Uuid;

// ============================================================================
// Configuration
// ============================================================================

/// Inbox configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InboxConfig {
    /// Poll interval for pulling messages (seconds)
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,

    /// Max retry attempts before dead-lettering
    #[serde(default = "default_max_retry")]
    pub max_retry: u32,

    /// Base delay for exponential backoff (ms)
    #[serde(default = "default_base_delay")]
    pub base_delay_ms: u64,

    /// Max delay cap (ms)
    #[serde(default = "default_max_delay")]
    pub max_delay_ms: u64,

    /// Jitter range (+/- ms)
    #[serde(default = "default_jitter")]
    pub jitter_ms: u64,

    /// Message send timeout (ms)
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,

    /// How long to keep acked messages (days)
    #[serde(default = "default_acked_ttl_days")]
    pub acked_ttl_days: i64,

    /// How long to keep dead letters (days)
    #[serde(default = "default_dead_letter_ttl_days")]
    pub dead_letter_ttl_days: i64,

    /// Alert webhook URL (optional)
    #[serde(default)]
    pub alert_webhook: Option<String>,
}

fn default_poll_interval() -> u64 {
    5
}
fn default_max_retry() -> u32 {
    3
}
fn default_base_delay() -> u64 {
    1000
}
fn default_max_delay() -> u64 {
    60000
}
fn default_jitter() -> u64 {
    500
}
fn default_timeout() -> u64 {
    10000
}
fn default_acked_ttl_days() -> i64 {
    7
}
fn default_dead_letter_ttl_days() -> i64 {
    30
}

impl Default for InboxConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_poll_interval(),
            max_retry: default_max_retry(),
            base_delay_ms: default_base_delay(),
            max_delay_ms: default_max_delay(),
            jitter_ms: default_jitter(),
            timeout_ms: default_timeout(),
            acked_ttl_days: default_acked_ttl_days(),
            dead_letter_ttl_days: default_dead_letter_ttl_days(),
            alert_webhook: None,
        }
    }
}

// ============================================================================
// Message Types
// ============================================================================

/// Message type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    /// Task message requiring explicit ack
    Task,
    /// Heartbeat/status sync (no persistence, no retry)
    Heartbeat,
    /// Lateral message between sibling agents
    Lateral,
}

/// Message status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Pending,
    Acked,
    DeadLetter,
}

/// Inbox message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxMessage {
    /// Unique message ID
    pub id: String,
    /// Sender agent ID
    pub from: String,
    /// Recipient agent ID
    pub to: String,
    /// Message type
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    /// Message payload (arbitrary JSON)
    pub payload: serde_json::Value,
    /// Current status
    pub status: MessageStatus,
    /// Current retry count
    pub retry_count: u32,
    /// Max retries before dead-lettering
    pub max_retry: u32,
    /// When created
    pub created_at: DateTime<Utc>,
    /// When acknowledged
    pub acked_at: Option<DateTime<Utc>>,
    /// When moved to dead letter
    pub dead_letter_at: Option<DateTime<Utc>>,
    /// When to retry next (if pending and retryable)
    pub next_retry_at: Option<DateTime<Utc>>,
    /// Last error if any
    pub last_error: Option<String>,
}

impl InboxMessage {
    /// Create a new pending message
    pub fn new(
        from: String,
        to: String,
        msg_type: MessageType,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from,
            to,
            msg_type,
            payload,
            status: MessageStatus::Pending,
            retry_count: 0,
            max_retry: 3,
            created_at: Utc::now(),
            acked_at: None,
            dead_letter_at: None,
            next_retry_at: None,
            last_error: None,
        }
    }

    /// Calculate next retry time using exponential backoff + jitter
    pub fn calculate_next_retry(&self, config: &InboxConfig) -> Option<DateTime<Utc>> {
        if self.retry_count >= self.max_retry {
            return None;
        }

        let base_ms = config.base_delay_ms * 2u64.pow(self.retry_count);
        let capped_ms = base_ms.min(config.max_delay_ms);

        // Add jitter: random in [-jitter_ms, +jitter_ms]
        let jitter_range = config.jitter_ms as i64;
        let jitter = if jitter_range > 0 {
            let jitter_val =
                (rand_jitter() % (jitter_range * 2) as u64) as i64 - jitter_range as i64;
            jitter_val
        } else {
            0
        };

        let total_ms = (capped_ms as i64 + jitter).max(0) as u64;
        Some(self.created_at + Duration::milliseconds(total_ms as i64))
    }

    /// Whether this message type should be persisted
    pub fn should_persist(&self) -> bool {
        matches!(self.msg_type, MessageType::Task | MessageType::Lateral)
    }

    /// Whether this message type should be retried
    pub fn should_retry(&self) -> bool {
        matches!(self.msg_type, MessageType::Task)
    }

    /// Mark as acknowledged
    pub fn ack(&mut self) {
        self.status = MessageStatus::Acked;
        self.acked_at = Some(Utc::now());
    }

    /// Move to dead letter
    pub fn dead_letter(&mut self, reason: &str) {
        self.status = MessageStatus::DeadLetter;
        self.dead_letter_at = Some(Utc::now());
        self.last_error = Some(reason.to_string());
    }
}

/// Simple pseudo-random jitter generator (0 to max-1)
fn rand_jitter() -> u64 {
    use std::time::Instant;
    // Mix thread id with current time for mild randomness
    let now = Instant::now();
    let ns = now.elapsed().as_nanos() as u64;
    ns % 1000
}

// ============================================================================
// Dead Letter Record
// ============================================================================

/// Dead letter log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterRecord {
    pub msg_id: String,
    pub original_msg: InboxMessage,
    pub failure_reason: String,
    pub last_error: Option<String>,
    pub retry_count: u32,
    pub dead_letter_at: DateTime<Utc>,
}

impl DeadLetterRecord {
    pub fn new(msg: InboxMessage, reason: &str) -> Self {
        let msg_id = msg.id.clone();
        let last_error = msg.last_error.clone();
        let retry_count = msg.retry_count;
        Self {
            msg_id,
            original_msg: msg,
            failure_reason: reason.to_string(),
            last_error,
            retry_count,
            dead_letter_at: Utc::now(),
        }
    }
}

// ============================================================================
// Communication Stats
// ============================================================================

/// Communication statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommStats {
    pub agent_id: String,
    pub pending_count: u64,
    pub acked_count: u64,
    pub dead_letter_count: u64,
    pub avg_latency_ms: Option<f64>,
    pub max_latency_ms: Option<u64>,
}

impl CommStats {
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            pending_count: 0,
            acked_count: 0,
            dead_letter_count: 0,
            avg_latency_ms: None,
            max_latency_ms: None,
        }
    }
}

// ============================================================================
// Inbox Manager
// ============================================================================

/// InboxManager - manages message inbox for an agent
pub struct InboxManager {
    /// Agent ID this inbox belongs to
    agent_id: String,
    /// Inbox configuration
    config: InboxConfig,
    /// In-memory cache of pending messages (loaded from disk)
    pending: RwLock<HashMap<String, InboxMessage>>,
    /// In-memory cache of dead letters
    dead_letters: RwLock<HashMap<String, DeadLetterRecord>>,
    /// Stats: acked count
    acked_count: RwLock<u64>,
    /// Stats: latency samples (in ms) for last hour
    latency_samples: RwLock<Vec<u64>>,
    /// Base path for inbox storage
    base_path: PathBuf,
}

impl InboxManager {
    /// Create a new InboxManager
    pub async fn new(agent_id: String, config: InboxConfig) -> std::io::Result<Self> {
        let base_path = Self::inbox_path(&agent_id);
        let manager = Self {
            agent_id,
            config,
            pending: RwLock::new(HashMap::new()),
            dead_letters: RwLock::new(HashMap::new()),
            acked_count: RwLock::new(0),
            latency_samples: RwLock::new(Vec::new()),
            base_path,
        };
        manager.ensure_directories().await?;
        manager.load_state().await?;
        Ok(manager)
    }

    /// Get the inbox base path
    fn inbox_path(agent_id: &str) -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".closeclaw/agents")
            .join(agent_id)
            .join("inbox")
    }

    /// Ensure all required directories exist
    async fn ensure_directories(&self) -> std::io::Result<()> {
        fs::create_dir_all(self.base_path.join("pending")).await?;
        fs::create_dir_all(self.base_path.join("acked")).await?;
        fs::create_dir_all(self.base_path.join("dead_letter")).await?;
        Ok(())
    }

    /// Load inbox state from disk (called on startup)
    async fn load_state(&self) -> std::io::Result<()> {
        // Load pending messages
        let pending_dir = self.base_path.join("pending");
        if pending_dir.exists() {
            let mut entries = fs::read_dir(&pending_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(content) = fs::read_to_string(&path).await {
                        if let Ok(msg) = serde_json::from_str::<InboxMessage>(&content) {
                            if msg.status == MessageStatus::Pending {
                                self.pending.write().await.insert(msg.id.clone(), msg);
                            }
                        }
                    }
                }
            }
        }

        // Load dead letters
        let dead_letter_dir = self.base_path.join("dead_letter");
        if dead_letter_dir.exists() {
            let mut entries = fs::read_dir(&dead_letter_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(content) = fs::read_to_string(&path).await {
                        if let Ok(record) = serde_json::from_str::<DeadLetterRecord>(&content) {
                            self.dead_letters
                                .write()
                                .await
                                .insert(record.msg_id.clone(), record);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Persist a message to disk
    async fn persist_message(&self, msg: &InboxMessage) -> std::io::Result<()> {
        let dir = match msg.status {
            MessageStatus::Pending => self.base_path.join("pending"),
            MessageStatus::Acked => self.base_path.join("acked"),
            MessageStatus::DeadLetter => self.base_path.join("dead_letter"),
        };
        let path = dir.join(format!("{}.json", msg.id));
        let json = serde_json::to_string_pretty(msg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(path, json).await
    }

    /// Remove a message from its current location
    async fn remove_message_file(
        &self,
        msg_id: &str,
        status: MessageStatus,
    ) -> std::io::Result<()> {
        let dir = match status {
            MessageStatus::Pending => self.base_path.join("pending"),
            MessageStatus::Acked => self.base_path.join("acked"),
            MessageStatus::DeadLetter => self.base_path.join("dead_letter"),
        };
        let path = dir.join(format!("{}.json", msg_id));
        if path.exists() {
            fs::remove_file(path).await?;
        }
        Ok(())
    }

    /// Push a new message to the inbox (called by sender)
    pub async fn push(&self, msg: InboxMessage) -> std::io::Result<()> {
        let msg_id = msg.id.clone();
        let status = msg.status;

        // Only persist if should be persisted
        if msg.should_persist() {
            self.persist_message(&msg).await?;
        }

        if status == MessageStatus::Pending {
            self.pending.write().await.insert(msg_id, msg);
        }
        Ok(())
    }

    /// Pull all pending messages for a recipient (called by recipient)
    /// Returns messages and marks them as acked
    pub async fn pull(&self, recipient_id: &str) -> std::io::Result<Vec<InboxMessage>> {
        let mut pending_guard = self.pending.write().await;
        let now = Utc::now();

        let mut to_deliver = Vec::new();
        let mut to_remove = Vec::new();

        for (id, msg) in pending_guard.iter_mut() {
            if msg.to == recipient_id && msg.status == MessageStatus::Pending {
                // Check if ready to deliver (not waiting for retry)
                if let Some(next_retry) = msg.next_retry_at {
                    if next_retry > now {
                        continue; // Still waiting for retry backoff
                    }
                }

                // Deliver this message
                msg.ack();
                to_deliver.push(msg.clone());
                to_remove.push(id.clone());

                // Update stats
                if let Some(acked_at) = msg.acked_at {
                    let latency_ms = (acked_at - msg.created_at).num_milliseconds() as u64;
                    let mut samples = self.latency_samples.write().await;
                    samples.push(latency_ms);
                    // Keep only last hour of samples (rough: keep last 720 samples at 5s interval)
                    if samples.len() > 720 {
                        samples.remove(0);
                    }
                }
            }
        }

        // Remove delivered messages from pending
        for id in &to_remove {
            if let Some(msg) = pending_guard.remove(id) {
                self.remove_message_file(id, MessageStatus::Pending).await?;
                self.persist_message(&msg).await?;
            }
        }

        *self.acked_count.write().await += to_deliver.len() as u64;

        Ok(to_deliver)
    }

    /// Acknowledge a specific message
    pub async fn ack(&self, msg_id: &str) -> std::io::Result<bool> {
        let mut pending_guard = self.pending.write().await;
        if let Some(msg) = pending_guard.get_mut(msg_id) {
            msg.ack();
            let msg_clone = msg.clone();
            drop(pending_guard);

            self.remove_message_file(msg_id, MessageStatus::Pending)
                .await?;
            self.persist_message(&msg_clone).await?;

            *self.acked_count.write().await += 1;

            // Update latency stats
            if let Some(acked_at) = msg_clone.acked_at {
                let latency_ms = (acked_at - msg_clone.created_at).num_milliseconds() as u64;
                self.latency_samples.write().await.push(latency_ms);
            }

            return Ok(true);
        }
        Ok(false)
    }

    /// Get communication statistics
    pub async fn get_stats(&self) -> CommStats {
        let pending_count = self.pending.read().await.len() as u64;
        let dead_letter_count = self.dead_letters.read().await.len() as u64;
        let acked_count = *self.acked_count.read().await;

        let samples = self.latency_samples.read().await;
        let (avg_latency, max_latency) = if samples.is_empty() {
            (None, None)
        } else {
            let sum: u64 = samples.iter().sum();
            let avg = sum as f64 / samples.len() as f64;
            let max = *samples.iter().max().unwrap_or(&0);
            (Some(avg), Some(max))
        };

        CommStats {
            agent_id: self.agent_id.clone(),
            pending_count,
            acked_count,
            dead_letter_count,
            avg_latency_ms: avg_latency,
            max_latency_ms: max_latency,
        }
    }

    /// Mark a message as dead letter (called after max retries exceeded)
    pub async fn mark_dead_letter(&self, msg_id: &str, reason: &str) -> std::io::Result<()> {
        let mut pending_guard = self.pending.write().await;
        if let Some(msg) = pending_guard.remove(msg_id) {
            let mut dead_msg = msg.clone();
            dead_msg.dead_letter(reason);

            let record = DeadLetterRecord::new(dead_msg.clone(), reason);

            // Persist dead letter
            self.remove_message_file(msg_id, MessageStatus::Pending)
                .await?;
            self.persist_message(&dead_msg).await?;

            // Store record
            self.dead_letters
                .write()
                .await
                .insert(msg_id.to_string(), record);

            // TODO: Send alert if webhook configured
            if let Some(ref webhook) = self.config.alert_webhook {
                tracing::warn!("Would send alert to webhook: {}", webhook);
            }

            Ok(())
        } else {
            Ok(())
        }
    }

    /// Garbage collect old messages
    pub async fn gc(&self) -> std::io::Result<u64> {
        let now = Utc::now();
        let acked_ttl = Duration::days(self.config.acked_ttl_days);
        let dead_letter_ttl = Duration::days(self.config.dead_letter_ttl_days);
        let mut removed = 0u64;

        // GC acked messages
        let acked_dir = self.base_path.join("acked");
        if acked_dir.exists() {
            let mut entries = fs::read_dir(&acked_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(content) = fs::read_to_string(&path).await {
                        if let Ok(msg) = serde_json::from_str::<InboxMessage>(&content) {
                            if let Some(acked_at) = msg.acked_at {
                                if now - acked_at > acked_ttl {
                                    fs::remove_file(path).await?;
                                    removed += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        // GC dead letters
        let dead_letter_dir = self.base_path.join("dead_letter");
        if dead_letter_dir.exists() {
            let mut entries = fs::read_dir(&dead_letter_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(content) = fs::read_to_string(&path).await {
                        if let Ok(record) = serde_json::from_str::<DeadLetterRecord>(&content) {
                            if now - record.dead_letter_at > dead_letter_ttl {
                                fs::remove_file(path).await?;
                                self.dead_letters.write().await.remove(&record.msg_id);
                                removed += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(removed)
    }

    /// Process pending retries (call periodically)
    pub async fn process_retries(&self) -> std::io::Result<()> {
        let now = Utc::now();
        let mut to_dead_letter = Vec::new();

        {
            let pending_guard = self.pending.write().await;
            for (id, msg) in pending_guard.iter() {
                if msg.status == MessageStatus::Pending && msg.should_retry() {
                    if let Some(next_retry) = msg.next_retry_at {
                        if next_retry <= now && msg.retry_count >= msg.max_retry {
                            to_dead_letter.push(id.clone());
                        }
                    }
                }
            }
        }

        // Mark dead letters
        for id in &to_dead_letter {
            self.mark_dead_letter(id, "max_retries_exceeded").await?;
        }

        Ok(())
    }

    /// Get configuration reference
    pub fn config(&self) -> &InboxConfig {
        &self.config
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> InboxConfig {
        InboxConfig {
            poll_interval_secs: 5,
            max_retry: 3,
            base_delay_ms: 1000,
            max_delay_ms: 60000,
            jitter_ms: 500,
            timeout_ms: 10000,
            acked_ttl_days: 7,
            dead_letter_ttl_days: 30,
            alert_webhook: None,
        }
    }

    #[test]
    fn test_message_creation() {
        let task_msg = InboxMessage::new(
            "parent-1".to_string(),
            "child-1".to_string(),
            MessageType::Task,
            serde_json::json!({"data": "test"}),
        );

        assert_eq!(task_msg.status, MessageStatus::Pending);
        assert_eq!(task_msg.retry_count, 0);
        assert!(task_msg.should_persist());
        assert!(task_msg.should_retry()); // Task should retry

        let heartbeat_msg = InboxMessage::new(
            "parent-1".to_string(),
            "child-1".to_string(),
            MessageType::Heartbeat,
            serde_json::json!({}),
        );
        assert!(!heartbeat_msg.should_persist()); // Heartbeat should not persist
        assert!(!heartbeat_msg.should_retry()); // Heartbeat should not retry
    }

    #[test]
    fn test_message_ack() {
        let mut msg = InboxMessage::new(
            "parent-1".to_string(),
            "child-1".to_string(),
            MessageType::Task,
            serde_json::json!({}),
        );
        msg.ack();
        assert_eq!(msg.status, MessageStatus::Acked);
        assert!(msg.acked_at.is_some());
    }

    #[test]
    fn test_message_dead_letter() {
        let mut msg = InboxMessage::new(
            "parent-1".to_string(),
            "child-1".to_string(),
            MessageType::Task,
            serde_json::json!({}),
        );
        msg.dead_letter("max_retries_exceeded");
        assert_eq!(msg.status, MessageStatus::DeadLetter);
        assert!(msg.dead_letter_at.is_some());
        assert_eq!(msg.last_error, Some("max_retries_exceeded".to_string()));
    }

    #[test]
    fn test_message_should_persist() {
        let task_msg = InboxMessage::new(
            "from".to_string(),
            "to".to_string(),
            MessageType::Task,
            serde_json::json!({}),
        );
        let heartbeat_msg = InboxMessage::new(
            "from".to_string(),
            "to".to_string(),
            MessageType::Heartbeat,
            serde_json::json!({}),
        );

        assert!(task_msg.should_persist());
        assert!(!heartbeat_msg.should_persist());
    }

    #[test]
    fn test_exponential_backoff() {
        let mut msg = InboxMessage::new(
            "from".to_string(),
            "to".to_string(),
            MessageType::Task,
            serde_json::json!({}),
        );
        msg.max_retry = 3;
        let config = test_config();

        // First retry
        let next = msg.calculate_next_retry(&config);
        assert!(next.is_some());
        // Delay should be around 1000ms +/- 500ms (so 500-1500ms)
        if let Some(t) = next {
            let delay = (t - msg.created_at).num_milliseconds();
            assert!(
                delay >= 500 && delay <= 1500,
                "Expected 500-1500ms, got {}",
                delay
            );
        }

        // Increment retry count and check again
        msg.retry_count = 1;
        let next = msg.calculate_next_retry(&config);
        assert!(next.is_some());
        if let Some(t) = next {
            let delay = (t - msg.created_at).num_milliseconds();
            // Should be around 2000ms +/- 500ms (so 1500-2500ms)
            assert!(
                delay >= 1500 && delay <= 2500,
                "Expected 1500-2500ms, got {}",
                delay
            );
        }
    }

    #[test]
    fn test_max_retry_exceeded() {
        let mut msg = InboxMessage::new(
            "from".to_string(),
            "to".to_string(),
            MessageType::Task,
            serde_json::json!({}),
        );
        msg.retry_count = 3; // Equal to max_retry
        msg.max_retry = 3;
        let config = test_config();

        let next = msg.calculate_next_retry(&config);
        assert!(next.is_none()); // No more retries
    }
}
