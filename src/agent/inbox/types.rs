//! Inbox data types and shared structures

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
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
pub fn rand_jitter() -> u64 {
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
