//! Inbox manager implementation

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;

use super::types::{CommStats, DeadLetterRecord, InboxConfig, InboxMessage, MessageStatus};

/// InboxManager - manages message inbox for an agent
pub struct InboxManager {
    /// Agent ID this inbox belongs to
    agent_id: String,
    /// Inbox configuration
    config: InboxConfig,
    /// In-memory cache of pending messages (loaded from disk)
    pending: tokio::sync::RwLock<HashMap<String, InboxMessage>>,
    /// In-memory cache of dead letters
    dead_letters: tokio::sync::RwLock<HashMap<String, DeadLetterRecord>>,
    /// Stats: acked count
    acked_count: tokio::sync::RwLock<u64>,
    /// Stats: latency samples (in ms) for last hour
    latency_samples: tokio::sync::RwLock<Vec<u64>>,
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
            pending: tokio::sync::RwLock::new(HashMap::new()),
            dead_letters: tokio::sync::RwLock::new(HashMap::new()),
            acked_count: tokio::sync::RwLock::new(0),
            latency_samples: tokio::sync::RwLock::new(Vec::new()),
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
