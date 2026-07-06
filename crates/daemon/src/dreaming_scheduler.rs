//! Dreaming Scheduler — background task for memory promotion and mining.
//!
//! Periodically triggers the dreaming pipeline (three-stage memory promotion)
//! followed by memory-mining (session transcript extraction). Spawned by the
//! daemon at startup and shut down gracefully via a `tokio::sync::watch`
//! channel.
//!
//! Supports cron-based scheduling via `DreamingConfig.schedule`. When a valid
//! cron expression is provided, the scheduler computes wall-clock next-fire
//! times. On parse failure it falls back to a fixed interval from
//! `SessionConfigProvider::dreaming_interval_secs()`.

use std::sync::Arc;

use chrono::{Datelike, Local, Timelike};
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tracing::{error, info};

use closeclaw_config::providers::MemoryConfigData;
use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::ConfigChangeEvent;
use closeclaw_config::ConfigManager;
use closeclaw_config::ConfigSection;
use closeclaw_memory::dreaming::DreamingPipeline;
use closeclaw_memory::miner::MemoryMiner;
use closeclaw_session::persistence::{PersistenceError, PersistenceService};

/// Errors that can occur during scheduler operations.
#[derive(Debug, Error)]
pub enum DreamingSchedulerError {
    /// Storage layer error.
    #[error("storage error: {0}")]
    Storage(#[from] PersistenceError),

    /// Dreaming pipeline error.
    #[error("dreaming error: {0}")]
    Dreaming(String),

    /// Memory miner error.
    #[error("miner error: {0}")]
    Miner(String),
}

/// Dreaming Scheduler — orchestrates dreaming pipeline and memory mining.
///
/// Follows the same background-task pattern as [`ArchiveSweeper`]:
/// `tokio::spawn` + `watch::Receiver` for shutdown coordination.
pub struct DreamingScheduler {
    storage: Arc<dyn PersistenceService>,
    config: Arc<dyn SessionConfigProvider>,
    dreaming_pipeline: Arc<DreamingPipeline>,
    memory_miner: Arc<MemoryMiner>,
    /// Optional cron expression from `DreamingConfig.schedule`.
    pub(crate) schedule: Option<String>,
    /// Config manager for hot-reload subscriptions.
    config_manager: Arc<ConfigManager>,
    /// Broadcast receiver for config change events.
    config_rx: tokio::sync::broadcast::Receiver<ConfigChangeEvent>,
    /// Optional channel receiver for immediate mining notifications
    /// from ArchiveSweeper (design doc §即时 hook).
    mining_notify_rx: Option<mpsc::Receiver<String>>,
}

impl DreamingScheduler {
    /// Create a new `DreamingScheduler`.
    pub fn new(
        storage: Arc<dyn PersistenceService>,
        config: Arc<dyn SessionConfigProvider>,
        dreaming_pipeline: Arc<DreamingPipeline>,
        memory_miner: Arc<MemoryMiner>,
        config_manager: Arc<ConfigManager>,
    ) -> Self {
        let config_rx = config_manager.subscribe_config_changes();
        Self {
            storage,
            config,
            dreaming_pipeline,
            memory_miner,
            schedule: None,
            config_manager,
            config_rx,
            mining_notify_rx: None,
        }
    }

    /// Set the cron schedule expression for dreaming.
    pub fn with_schedule(mut self, schedule: Option<String>) -> Self {
        self.schedule = schedule;
        self
    }

    /// Attach the mining notify channel receiver.  When a session is
    /// archived by the [`ArchiveSweeper`], the sender emits the
    /// session ID through this channel so mining starts immediately.
    pub fn with_mining_notify_rx(mut self, rx: mpsc::Receiver<String>) -> Self {
        self.mining_notify_rx = Some(rx);
        self
    }

    /// Handle a config change for the Memory section.
    async fn handle_config_change(&mut self) {
        match self.config_manager.section(ConfigSection::Memory) {
            Some(value) => {
                let content = match serde_json::to_string(&value) {
                    Ok(s) => s,
                    Err(e) => {
                        error!(%e, "failed to serialize memory config");
                        return;
                    }
                };
                let memory_config = match MemoryConfigData::from_json_str(&content) {
                    Ok(c) => c,
                    Err(e) => {
                        error!(%e, "failed to parse memory config");
                        return;
                    }
                };
                self.dreaming_pipeline
                    .update_config(memory_config.config.dreaming.clone());
                self.memory_miner.update_config(
                    closeclaw_memory::miner::MinerConfig::from_mining_config(
                        &memory_config.config.mining,
                    ),
                );
                self.schedule = memory_config.config.dreaming.schedule.clone();
                info!("dreaming config reloaded via config manager");
            }
            None => {
                error!("Memory config section not available");
            }
        }
    }

    /// Run the scheduler loop until `shutdown` signal is received.
    ///
    /// Uses cron-based scheduling when a valid expression is available;
    /// falls back to a fixed interval otherwise. Each cycle: dreaming
    /// pipeline first, then mining scan.
    pub async fn run(&mut self, mut shutdown: watch::Receiver<()>) {
        let cron_schedule = self.schedule.as_deref().and_then(parse_cron_schedule);

        if let Some(ref sched) = cron_schedule {
            let next = sched.next_fire_time(Local::now());
            let delay = (next - Local::now())
                .to_std()
                .unwrap_or_else(|_| tokio::time::Duration::from_secs(1));
            info!(
                schedule = %sched.display(),
                "DreamingScheduler started with cron schedule"
            );
            self.run_cron_loop(&mut shutdown, sched, delay).await;
        } else {
            let interval_secs = self.config.dreaming_interval_secs();
            let interval = tokio::time::Duration::from_secs(interval_secs);
            info!(
                "DreamingScheduler started with fixed interval {}s",
                interval_secs
            );
            self.run_fixed_interval_loop(&mut shutdown, interval).await;
        }
    }

    /// Cron-based scheduling loop: compute next fire time from wall clock.
    async fn run_cron_loop(
        &mut self,
        shutdown: &mut watch::Receiver<()>,
        schedule: &CronSchedule,
        initial_delay: tokio::time::Duration,
    ) {
        let mut next_fire = tokio::time::Instant::now() + initial_delay;

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!(
                        "DreamingScheduler received shutdown signal, exiting"
                    );
                    break;
                }
                result = self.config_rx.recv() => {
                    if self.is_memory_config_reload(result) {
                        self.handle_cron_config_reload(
                            &mut next_fire,
                        ).await;
                    }
                }
                Some(session_id) = async {
                    match &mut self.mining_notify_rx {
                        Some(rx) => rx.recv().await,
                        None => None,
                    }
                } => {
                    let agents = self.config.list_agents();
                    if !agents.is_empty() {
                        info!(
                            session_id = %session_id,
                            "mining triggered by archive notification"
                        );
                        self.mine_session(&session_id, &agents).await;
                    }
                }
                _ = tokio::time::sleep_until(next_fire) => {
                    if let Err(e) = self.run_once().await {
                        error!(
                            %e,
                            "DreamingScheduler run_once returned error, \
                             continuing loop"
                        );
                    }
                    let next =
                        schedule.next_fire_time(Local::now());
                    let delay = (next - Local::now())
                        .to_std()
                        .unwrap_or_else(|_| {
                            tokio::time::Duration::from_secs(1)
                        });
                    next_fire =
                        tokio::time::Instant::now() + delay;
                }
            }
        }
    }

    /// Check if the event is a Memory config reload.
    fn is_memory_config_reload(
        &self,
        result: Result<ConfigChangeEvent, tokio::sync::broadcast::error::RecvError>,
    ) -> bool {
        matches!(
            result,
            Ok(ConfigChangeEvent::Reloaded {
                section: ConfigSection::Memory,
            })
        )
    }

    /// Handle a config reload within the cron loop: apply new config,
    /// re-parse the schedule, and recompute next fire time.
    async fn handle_cron_config_reload(&mut self, next_fire: &mut tokio::time::Instant) {
        self.handle_config_change().await;
        let new_sched = self.schedule.as_deref().and_then(parse_cron_schedule);
        if let Some(ref new_sched) = new_sched {
            let next = new_sched.next_fire_time(Local::now());
            let delay = (next - Local::now())
                .to_std()
                .unwrap_or_else(|_| tokio::time::Duration::from_secs(1));
            *next_fire = tokio::time::Instant::now() + delay;
            info!(
                schedule = %new_sched.display(),
                "schedule updated after config reload"
            );
        } else {
            let interval_secs = self.config.dreaming_interval_secs();
            let interval = tokio::time::Duration::from_secs(interval_secs);
            info!(
                "schedule became invalid after config \
                 reload, falling back to fixed \
                 interval {}s",
                interval_secs
            );
            *next_fire = tokio::time::Instant::now() + interval;
        }
    }

    /// Fixed-interval fallback scheduling loop.
    async fn run_fixed_interval_loop(
        &mut self,
        shutdown: &mut watch::Receiver<()>,
        interval: tokio::time::Duration,
    ) {
        let mut next_fire = tokio::time::Instant::now() + interval;

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    info!(
                        "DreamingScheduler received shutdown signal, exiting"
                    );
                    break;
                }
                result = self.config_rx.recv() => {
                    if self.is_memory_config_reload(result) {
                        self.handle_config_change().await;
                    }
                }
                Some(session_id) = async {
                    match &mut self.mining_notify_rx {
                        Some(rx) => rx.recv().await,
                        None => None,
                    }
                } => {
                    let agents = self.config.list_agents();
                    if !agents.is_empty() {
                        info!(
                            session_id = %session_id,
                            "mining triggered by archive notification"
                        );
                        self.mine_session(&session_id, &agents).await;
                    }
                }
                _ = tokio::time::sleep_until(next_fire) => {
                    if let Err(e) = self.run_once().await {
                        error!(
                            %e,
                            "DreamingScheduler run_once returned error, \
                             continuing loop"
                        );
                    }
                    next_fire =
                        tokio::time::Instant::now() + interval;
                }
            }
        }
    }

    /// Execute one cycle: dreaming first, then mining.
    pub async fn run_once(&self) -> Result<(), DreamingSchedulerError> {
        let agents = self.config.list_agents();
        if agents.is_empty() {
            return Ok(());
        }

        // Step 1: Run dreaming pipeline (process already-mined entries)
        if let Err(e) = self.dreaming_pipeline.run_once(self.storage.as_ref()).await {
            error!(%e, "dreaming pipeline failed");
            // Continue to mining even if dreaming fails
        }

        // Step 2: Run mining scan (extract entries from new archived
        // sessions)
        let unmined = self.storage.list_archived_unmined_sessions().await?;

        for session_id in unmined {
            self.mine_session(&session_id, &agents).await;
        }

        Ok(())
    }

    /// Mine a single archived session: load checkpoint, filter by agent,
    /// format transcript, and invoke the memory miner.
    async fn mine_session(&self, session_id: &str, agents: &[String]) {
        let checkpoint = match self.storage.load_archived_checkpoint(session_id).await {
            Ok(Some(cp)) => cp,
            Ok(None) => {
                error!(
                    session_id = %session_id,
                    "archived checkpoint not found, skipping"
                );
                return;
            }
            Err(e) => {
                error!(
                    session_id = %session_id,
                    %e,
                    "failed to load archived checkpoint"
                );
                return;
            }
        };

        if let Some(ref aid) = checkpoint.agent_id {
            if !agents.contains(aid) {
                return;
            }
        }

        let raw_transcript = format_transcript(&checkpoint.pending_messages);

        if let Err(e) = self
            .memory_miner
            .mine_session_from_checkpoint(
                session_id,
                &raw_transcript,
                &checkpoint,
                self.storage.as_ref(),
            )
            .await
        {
            error!(
                session_id = %session_id,
                %e,
                "failed to mine session"
            );
        }
    }
}

/// Format pending messages into the raw transcript text expected by the
/// miner.
///
/// Messages are rendered as `"<role>: <content>"` lines, matching the
/// format produced by session transcript recording.
fn format_transcript(messages: &[closeclaw_session::persistence::PendingMessage]) -> String {
    messages
        .iter()
        .map(|m| {
            let role = m
                .role
                .as_deref()
                .filter(|r| !r.is_empty())
                .unwrap_or("unknown");
            format!("{role}: {}", m.content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl std::fmt::Debug for DreamingScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DreamingScheduler")
            .field("schedule", &self.schedule)
            .field("config_manager", &"<ConfigManager>")
            .finish()
    }
}

// ── Cron parsing ──────────────────────────────────────────────────────

/// Parsed cron schedule (minute, hour, day-of-month, month,
/// day-of-week).
///
/// Supports `*` for "any" and specific numeric values.
#[derive(Debug, Clone)]
struct CronSchedule {
    minute: CronField,
    hour: CronField,
    dom: CronField,
    month: CronField,
    dow: CronField,
}

/// A single cron field — either `*` (any) or a specific numeric value.
#[derive(Debug, Clone)]
enum CronField {
    Any,
    Value(u32),
}

/// Parse a 5-field cron expression into a [`CronSchedule`].
///
/// Only supports `*` and non-negative integer values. Returns `None` on
/// any parse error, allowing the caller to fall back to fixed-interval.
fn parse_cron_schedule(expr: &str) -> Option<CronSchedule> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }
    Some(CronSchedule {
        minute: parse_cron_field(parts[0], 0, 59)?,
        hour: parse_cron_field(parts[1], 0, 23)?,
        dom: parse_cron_field(parts[2], 1, 31)?,
        month: parse_cron_field(parts[3], 1, 12)?,
        dow: parse_cron_field(parts[4], 0, 6)?,
    })
}

/// Parse a single cron field into a [`CronField`].
fn parse_cron_field(s: &str, min: u32, max: u32) -> Option<CronField> {
    if s == "*" {
        return Some(CronField::Any);
    }
    let val: u32 = s.parse().ok()?;
    if val < min || val > max {
        return None;
    }
    Some(CronField::Value(val))
}

impl CronSchedule {
    /// Compute the next fire time strictly after `now`.
    ///
    /// Iterates minute-by-minute (bounded to avoid infinite loops) to find
    /// the next datetime matching all cron fields.
    fn next_fire_time(&self, now: chrono::DateTime<Local>) -> chrono::DateTime<Local> {
        // Start from the next whole minute after now.
        let mut candidate = now + chrono::Duration::minutes(1);
        candidate = candidate.with_second(0).unwrap_or(candidate);
        candidate = candidate.with_nanosecond(0).unwrap_or(candidate);

        // Cap iterations to prevent infinite loops (max ~2 days of
        // minutes).
        for _ in 0..2880 {
            if self.matches(&candidate) {
                return candidate;
            }
            candidate += chrono::Duration::minutes(1);
        }

        // Fallback: return now + 1 hour (should never reach here
        // for valid cron expressions).
        now + chrono::Duration::hours(1)
    }

    /// Check if a datetime matches all cron fields.
    fn matches(&self, dt: &chrono::DateTime<Local>) -> bool {
        match_field(&self.minute, dt.minute())
            && match_field(&self.hour, dt.hour())
            && match_field(&self.dom, dt.day())
            && match_field(&self.month, dt.month())
            && match_field(&self.dow, dt.weekday().num_days_from_sunday())
    }

    /// Human-readable display for logging.
    fn display(&self) -> String {
        format!(
            "{} {} {} {} {}",
            display_field(&self.minute),
            display_field(&self.hour),
            display_field(&self.dom),
            display_field(&self.month),
            display_field(&self.dow),
        )
    }
}

/// Check if a numeric value matches a [`CronField`].
fn match_field(field: &CronField, value: u32) -> bool {
    match field {
        CronField::Any => true,
        CronField::Value(v) => *v == value,
    }
}

/// Display a cron field for logging.
fn display_field(field: &CronField) -> String {
    match field {
        CronField::Any => "*".to_string(),
        CronField::Value(v) => v.to_string(),
    }
}

// ── Transcript format tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_transcript_empty() {
        let result = format_transcript(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_transcript_converts_messages() {
        use closeclaw_session::persistence::PendingMessage;
        let messages = vec![
            PendingMessage::with_role("msg1".into(), "hello".into(), "user".into()),
            PendingMessage::with_role("msg2".into(), "hi there".into(), "assistant".into()),
        ];
        let result = format_transcript(&messages);
        assert!(result.contains("user: hello"));
        assert!(result.contains("assistant: hi there"));
    }

    #[test]
    fn test_format_transcript_handles_empty_role() {
        use closeclaw_session::persistence::PendingMessage;
        let messages = vec![PendingMessage::new("msg1".into(), "content".into())];
        let result = format_transcript(&messages);
        assert!(result.contains("unknown: content"));
    }

    #[test]
    fn test_format_transcript_uses_role_not_message_id() {
        use closeclaw_session::persistence::PendingMessage;
        let messages = vec![
            PendingMessage::with_role("out-12345".into(), "hello".into(), "assistant".into()),
            PendingMessage::with_role("pending-67890".into(), "world".into(), "user".into()),
        ];
        let result = format_transcript(&messages);
        assert!(result.contains("assistant: hello"));
        assert!(result.contains("user: world"));
        assert!(!result.contains("out-12345"));
        assert!(!result.contains("pending-67890"));
    }
}

// ── Cron tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod cron_tests {
    use super::*;

    #[test]
    fn test_parse_cron_valid() {
        let sched = parse_cron_schedule("0 3 * * *").unwrap();
        assert!(matches!(sched.minute, CronField::Value(0)));
        assert!(matches!(sched.hour, CronField::Value(3)));
        assert!(matches!(sched.dom, CronField::Any));
        assert!(matches!(sched.month, CronField::Any));
        assert!(matches!(sched.dow, CronField::Any));
    }

    #[test]
    fn test_parse_cron_invalid_parts() {
        assert!(parse_cron_schedule("0 3 * *").is_none());
        assert!(parse_cron_schedule("0 3 * * * *").is_none());
        assert!(parse_cron_schedule("").is_none());
    }

    #[test]
    fn test_parse_cron_invalid_values() {
        assert!(parse_cron_schedule("60 3 * * *").is_none());
        assert!(parse_cron_schedule("0 24 * * *").is_none());
        assert!(parse_cron_schedule("0 3 0 * *").is_none());
        assert!(parse_cron_schedule("0 3 * 13 *").is_none());
    }

    #[test]
    fn test_parse_cron_non_numeric() {
        assert!(parse_cron_schedule("abc 3 * * *").is_none());
    }

    #[test]
    fn test_next_fire_daily_at_3am() {
        let sched = parse_cron_schedule("0 3 * * *").unwrap();
        // 2024-07-06 10:00 — next fire should be 2024-07-07 03:00
        let now = chrono::NaiveDate::from_ymd_opt(2024, 7, 6)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        let next = sched.next_fire_time(now);
        assert_eq!(next.hour(), 3);
        assert_eq!(next.minute(), 0);
        assert!(next > now);
    }

    #[test]
    fn test_next_fire_before_3am() {
        let sched = parse_cron_schedule("0 3 * * *").unwrap();
        // 2024-07-06 02:00 — next fire should be same day 03:00
        let now = chrono::NaiveDate::from_ymd_opt(2024, 7, 6)
            .unwrap()
            .and_hms_opt(2, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        let next = sched.next_fire_time(now);
        assert_eq!(next.hour(), 3);
        assert_eq!(next.minute(), 0);
        assert_eq!(next.date_naive(), now.date_naive());
    }

    #[test]
    fn test_next_fire_exact_3am() {
        let sched = parse_cron_schedule("0 3 * * *").unwrap();
        // At exactly 3:00 → next should be tomorrow 3:00
        let now = chrono::NaiveDate::from_ymd_opt(2024, 7, 6)
            .unwrap()
            .and_hms_opt(3, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        let next = sched.next_fire_time(now);
        assert_eq!(next.hour(), 3);
        assert_eq!(next.minute(), 0);
        assert!(next > now);
    }

    #[test]
    fn test_matches_star_matches_all() {
        let field = CronField::Any;
        assert!(match_field(&field, 0));
        assert!(match_field(&field, 59));
    }

    #[test]
    fn test_matches_value_only_exact() {
        let field = CronField::Value(42);
        assert!(match_field(&field, 42));
        assert!(!match_field(&field, 43));
        assert!(!match_field(&field, 41));
    }

    #[test]
    fn test_display_star() {
        assert_eq!(display_field(&CronField::Any), "*");
    }

    #[test]
    fn test_display_value() {
        assert_eq!(display_field(&CronField::Value(5)), "5");
    }

    #[test]
    fn test_schedule_display() {
        let sched = parse_cron_schedule("0 3 * * *").unwrap();
        assert_eq!(sched.display(), "0 3 * * *");
    }
}
