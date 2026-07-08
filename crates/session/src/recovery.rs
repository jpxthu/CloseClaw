//! Session recovery service
//!
//! Provides functionality to recover sessions from persisted checkpoints
//! during gateway startup, including spawn_tree reconstruction.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::llm_session::PROGRESS_APPEND_PREFIX;
use crate::persistence::{
    ApprovalToolCallRecord, PersistenceError, PersistenceService, ProgressToolCallRecord,
    SessionCheckpoint,
};

/// Prefix marker for approval history entries in `system_appends`.
///
/// When injected as a fallback (layer 3), this prefix tags the entry
/// so it can be identified in subsequent recovery scans.
pub const APPROVAL_HISTORY_PREFIX: &str = "__approval_history__:";

/// Prefix marker for plan references extracted from message history
/// in `system_appends`.
///
/// When injected as a fallback (layer 4), this prefix tags the entry
/// so it can be identified in subsequent recovery scans.
pub const PLAN_REFERENCES_PREFIX: &str = "__plan_references__:";

/// Recovery report containing results of the recovery process
#[derive(Debug)]
pub struct RecoveryReport {
    /// List of session IDs that were successfully recovered
    pub recovered: Vec<String>,
    /// List of session IDs that failed to recover
    pub failed: Vec<String>,
    /// Spawn tree reconstructed from recovered checkpoints
    pub spawn_tree: SpawnTree,
    /// List of session IDs that had pending operations (dirty sessions)
    pub dirty_sessions: Vec<String>,
}

impl RecoveryReport {
    /// Returns true if all sessions were recovered successfully
    pub fn is_full_success(&self) -> bool {
        self.failed.is_empty()
    }

    /// Returns the total number of sessions processed
    pub fn total(&self) -> usize {
        self.recovered.len() + self.failed.len()
    }
}

/// Session recovery service — recovers sessions from persisted checkpoints
pub struct SessionRecoveryService<S: PersistenceService + ?Sized> {
    storage: Arc<S>,
    /// Callback to restore a session from checkpoint
    /// The closure receives the session_id and checkpoint, and should restore the session state.
    #[allow(clippy::type_complexity)]
    restore_fn: RwLock<
        Option<
            Box<
                dyn Fn(
                        &str,
                        &SessionCheckpoint,
                        Option<&str>,
                        &[String],
                    ) -> Result<(), PersistenceError>
                    + Send
                    + Sync,
            >,
        >,
    >,
}

impl<S: PersistenceService + ?Sized> SessionRecoveryService<S> {
    /// Create a new SessionRecoveryService
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage,
            restore_fn: RwLock::new(None),
        }
    }

    /// Set the restore callback
    ///
    /// The callback will be invoked for each session during recovery.
    /// It receives the session_id, checkpoint, optional recovery notification
    /// text, and any tool failure result strings to inject.
    pub async fn set_restore_callback<F>(&self, callback: F)
    where
        F: Fn(&str, &SessionCheckpoint, Option<&str>, &[String]) -> Result<(), PersistenceError>
            + Send
            + Sync
            + 'static,
    {
        let mut restore_fn = self.restore_fn.write().await;
        *restore_fn = Some(Box::new(callback));
    }

    /// 执行恢复流程
    ///
    /// 扫描 storage 中所有 active session 并逐一恢复。恢复后根据 checkpoint
    /// 数据重建 spawn_tree：
    /// - 有 `parent_session_id` 且父 session 也已恢复 → 注册为父节点的子节点
    /// - 有 `parent_session_id` 但父 session 未恢复（已被 sweep）→ 降级为根节点，depth 重置为 0
    /// - 无 `parent_session_id` → 确认为根节点
    pub async fn recover(&self) -> Result<RecoveryReport, PersistenceError> {
        let active_sessions = self.storage.list_active_sessions().await?;
        let mut recovered = Vec::new();
        let mut failed = Vec::new();
        let mut checkpoints: HashMap<String, SessionCheckpoint> = HashMap::new();

        for session_id in &active_sessions {
            match self.recover_session(session_id).await {
                Ok(()) => {
                    recovered.push(session_id.clone());
                    // Load checkpoint for spawn_tree reconstruction
                    if let Ok(Some(cp)) = self.storage.load_checkpoint(session_id).await {
                        checkpoints.insert(session_id.clone(), cp);
                    }
                }
                Err(e) => {
                    tracing::error!(
                        session_id = %session_id,
                        "Failed to recover session: {}",
                        e
                    );
                    failed.push(session_id.clone());
                }
            }
        }

        // Scan archived sessions for pending operations (defensive scan)
        let archived_sessions = match self.storage.list_archived_sessions().await {
            Ok(sessions) => sessions,
            Err(e) => {
                tracing::error!("Failed to list archived sessions: {}", e);
                Vec::new()
            }
        };

        for session_id in &archived_sessions {
            // Skip if already recovered as active session
            if recovered.contains(session_id) {
                continue;
            }
            match self.storage.load_archived_checkpoint(session_id).await {
                Ok(Some(cp)) => {
                    if cp.pending_operations.is_empty() {
                        // Clean archived session — skip
                        continue;
                    }
                    // Restore archived checkpoint to active state
                    if let Err(e) = self.storage.restore_checkpoint(session_id).await {
                        tracing::error!(
                            session_id = %session_id,
                            "Failed to restore archived session: {}",
                            e
                        );
                        failed.push(session_id.clone());
                        continue;
                    }
                    // Load the restored checkpoint — plan file content and
                    // recovery notifications will be injected in the unified
                    // loop after all sessions are collected.
                    match self.storage.load_checkpoint(session_id).await {
                        Ok(Some(restored_cp)) => {
                            checkpoints.insert(session_id.clone(), restored_cp);
                            recovered.push(session_id.clone());
                        }
                        Ok(None) => {
                            tracing::warn!(
                                session_id = %session_id,
                                "Restored checkpoint not found"
                            );
                            failed.push(session_id.clone());
                        }
                        Err(e) => {
                            tracing::error!(
                                session_id = %session_id,
                                "Failed to load restored checkpoint: {}",
                                e
                            );
                            failed.push(session_id.clone());
                        }
                    }
                }
                Ok(None) => {
                    tracing::warn!(
                        session_id = %session_id,
                        "Archived session checkpoint not found"
                    );
                    failed.push(session_id.clone());
                }
                Err(e) => {
                    tracing::error!(
                        session_id = %session_id,
                        "Failed to load archived checkpoint: {}",
                        e
                    );
                    failed.push(session_id.clone());
                }
            }
        }

        // Collect dirty sessions (those with pending operations)
        let dirty_sessions: Vec<String> = checkpoints
            .iter()
            .filter(|(_, cp)| !cp.pending_operations.is_empty())
            .map(|(id, _)| id.clone())
            .collect();

        // Inject plan file content, progress history fallback, and recovery
        // notifications for all recovered sessions.
        for session_id in &recovered {
            if let Some(cp) = checkpoints.get_mut(session_id) {
                // Layer 3: inject approval tool call history into system_appends
                self.inject_approval_history(session_id, cp);
                // Layer 4: fallback — inject plan references from message history
                // when layers 1–3 are all unavailable
                self.inject_plan_references(session_id, cp);
                // Inject recovery notifications for dirty sessions
                if !cp.pending_operations.is_empty() {
                    self.inject_recovery_notifications(session_id, cp);
                }
            }
        }

        // Persist checkpoints with injected plan file content and notifications
        for session_id in &recovered {
            if let Some(cp) = checkpoints.get(session_id) {
                if let Err(e) = self.storage.save_checkpoint(cp).await {
                    tracing::error!(
                        session_id = %session_id,
                        "Failed to persist checkpoint with injected content: {}",
                        e
                    );
                }
            }
        }

        let (spawn_tree, demoted) = Self::build_spawn_tree(&mut checkpoints, &recovered);

        // 持久化降级后的 checkpoint（depth 重置为 0）
        for session_id in &demoted {
            if let Some(cp) = checkpoints.get(session_id) {
                if let Err(e) = self.storage.save_checkpoint(cp).await {
                    tracing::error!(
                        session_id = %session_id,
                        "Failed to persist demoted checkpoint: {}",
                        e
                    );
                }
            }
        }

        Ok(RecoveryReport {
            recovered,
            failed,
            spawn_tree,
            dirty_sessions,
        })
    }

    /// Inject approval tool call history into checkpoint's system_appends (layer 3).
    ///
    /// When a checkpoint has `approval_tool_calls`, this method formats them
    /// and adds them to `system_appends` with [`APPROVAL_HISTORY_PREFIX`]
    /// so the session can recover plan context from approval records.
    ///
    /// If no approval tool calls exist, the checkpoint is left unchanged
    /// (graceful degradation to layer 2 only).
    fn inject_approval_history(&self, session_id: &str, checkpoint: &mut SessionCheckpoint) {
        if checkpoint.approval_tool_calls.is_empty() {
            return;
        }

        let summary = format_approval_history(&checkpoint.approval_tool_calls);
        if summary.is_empty() {
            return;
        }

        let tagged = format!("{}{}", APPROVAL_HISTORY_PREFIX, summary);

        // Replace existing entry in-place if present, otherwise append.
        if let Some(slot) = checkpoint
            .system_appends
            .iter_mut()
            .find(|s| s.starts_with(APPROVAL_HISTORY_PREFIX))
        {
            *slot = tagged;
        } else {
            checkpoint.system_appends.push(tagged);
        }

        tracing::info!(
            session_id = %session_id,
            call_count = checkpoint.approval_tool_calls.len(),
            "injected approval tool call history into system_appends"
        );
    }

    /// Inject plan references from session message history into checkpoint's
    /// system_appends (layer 4 fallback).
    ///
    /// When the first three layers are all unavailable — no `plan_state`
    /// (layer 1), no approval history in `system_appends` (layer 2), and no
    /// approval tool call history injected (layer 3) — this method checks
    /// `plan_references` in the checkpoint. If non-empty, it builds a
    /// summary and adds it to `system_appends` with
    /// [`PLAN_REFERENCES_PREFIX`].
    ///
    /// The trigger condition is explicit: only when layers 1–3 are all
    /// unavailable.
    fn inject_plan_references(&self, session_id: &str, checkpoint: &mut SessionCheckpoint) {
        // Layer 1: Only trigger when plan_state is unavailable
        if checkpoint.plan_state.is_some() {
            return;
        }

        // Layer 3: Only trigger when approval history was NOT injected
        // (i.e., no APPROVAL_HISTORY_PREFIX entry exists)
        let has_approval_history = checkpoint
            .system_appends
            .iter()
            .any(|s| s.starts_with(APPROVAL_HISTORY_PREFIX));
        if has_approval_history {
            return;
        }

        // Layer 2: Only trigger when progress summary was NOT injected
        // (i.e., no PROGRESS_APPEND_PREFIX entry exists in system_appends)
        let has_progress_summary = checkpoint
            .system_appends
            .iter()
            .any(|s| s.starts_with(PROGRESS_APPEND_PREFIX));
        if has_progress_summary {
            return;
        }

        // Only trigger when plan_references is non-empty
        if checkpoint.plan_references.is_empty() {
            return;
        }

        let summary = format_plan_references(&checkpoint.plan_references);
        if summary.is_empty() {
            return;
        }

        let tagged = format!("{}{}", PLAN_REFERENCES_PREFIX, summary);

        // Replace existing entry in-place if present, otherwise append.
        if let Some(slot) = checkpoint
            .system_appends
            .iter_mut()
            .find(|s| s.starts_with(PLAN_REFERENCES_PREFIX))
        {
            *slot = tagged;
        } else {
            checkpoint.system_appends.push(tagged);
        }

        tracing::info!(
            session_id = %session_id,
            ref_count = checkpoint.plan_references.len(),
            "layer 4 fallback: injected plan references into system_appends"
        );
    }

    /// Recover a single session
    async fn recover_session(&self, session_id: &str) -> Result<(), PersistenceError> {
        let checkpoint = self
            .storage
            .load_checkpoint(session_id)
            .await?
            .ok_or_else(|| PersistenceError::NotFound(session_id.to_string()))?;

        // Use the pre-stored recovery_notification from the checkpoint
        // (set by inject_recovery_notifications) when available;
        // otherwise fall back to building it fresh.
        let notification = if let Some(ref stored) = checkpoint.recovery_notification {
            Some(stored.clone())
        } else if !checkpoint.pending_operations.is_empty() {
            Some(self.build_notification_text(&checkpoint))
        } else {
            None
        };

        // Use pre-stored pending_tool_failures when available;
        // otherwise build them fresh.
        let tool_failures = if !checkpoint.pending_tool_failures.is_empty() {
            checkpoint.pending_tool_failures.clone()
        } else {
            self.build_tool_failure_results(&checkpoint)
        };

        let restore_fn = self.restore_fn.read().await;
        if let Some(callback) = restore_fn.as_ref() {
            callback(
                session_id,
                &checkpoint,
                notification.as_deref(),
                &tool_failures,
            )?;
        }

        Ok(())
    }

    /// Build the notification text for a dirty session.
    ///
    /// Stores the recovery notification and tool failure results in the
    /// checkpoint so they can be read back when sessions are restored.
    fn inject_recovery_notifications(&self, session_id: &str, checkpoint: &mut SessionCheckpoint) {
        if checkpoint.pending_operations.is_empty() {
            return;
        }

        let notification = self.build_notification_text(checkpoint);
        checkpoint.recovery_notification = Some(notification);
        checkpoint.pending_tool_failures = self.build_tool_failure_results(checkpoint);

        tracing::info!(
            session_id = %session_id,
            pending_count = checkpoint.pending_operations.len(),
            "storing recovery notification in checkpoint"
        );
    }

    /// Build notification text listing pending operations.
    fn build_notification_text(&self, checkpoint: &SessionCheckpoint) -> String {
        use crate::persistence::PendingOperationType;

        let restart_time = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");

        // Build summary by op_type
        let mut tool_calls = Vec::new();
        let mut sub_spawns = Vec::new();
        let mut outbound_msgs = Vec::new();

        for op in &checkpoint.pending_operations {
            match op.op_type {
                PendingOperationType::ToolCall => {
                    tool_calls.push(format!(
                        "  • 工具调用: {}({}) — 发起于 {}",
                        op.name,
                        if op.args.is_empty() {
                            "无参数".to_string()
                        } else {
                            op.args.clone()
                        },
                        op.created_at.format("%Y-%m-%dT%H:%M:%SZ")
                    ));
                }
                PendingOperationType::SubSessionSpawn => {
                    sub_spawns.push(format!(
                        "  • 子 Session: {} — 发起于 {}",
                        op.name,
                        op.created_at.format("%Y-%m-%dT%H:%M:%SZ")
                    ));
                }
                PendingOperationType::OutboundMessage => {
                    outbound_msgs.push(format!(
                        "  • 出站消息: {} — 创建于 {}",
                        op.name,
                        op.created_at.format("%Y-%m-%dT%H:%M:%SZ")
                    ));
                }
            }
        }

        let mut sections = Vec::new();
        if !tool_calls.is_empty() {
            sections.push(tool_calls.join("\n"));
        }
        if !sub_spawns.is_empty() {
            sections.push(sub_spawns.join("\n"));
        }
        if !outbound_msgs.is_empty() {
            sections.push(outbound_msgs.join("\n"));
        }

        format!(
            "[系统] 网关已重启（重启时间: {restart_time}）\n\n\
以下操作在重启前未完成：\n{ops}\n\n\
你可以使用 sessions_list、sessions_history、process 等工具\n\
了解当前状态，自行判断这些操作的结果，并决定后续处理。",
            restart_time = restart_time,
            ops = sections.join("\n\n"),
        )
    }

    /// Build tool failure result strings for pending tool call operations.
    fn build_tool_failure_results(&self, checkpoint: &SessionCheckpoint) -> Vec<String> {
        use crate::persistence::PendingOperationType;

        checkpoint
            .pending_operations
            .iter()
            .filter(|op| op.op_type == PendingOperationType::ToolCall)
            .map(|op| {
                serde_json::json!({
                    "error": "进程中断：网关重启",
                    "tool": op.name,
                    "op_id": op.op_id,
                })
                .to_string()
            })
            .collect()
    }

    /// 根据已恢复 session 的 checkpoint 构建 spawn_tree。
    ///
    /// - 有 `parent_session_id` 且父 session 已恢复 → 注册为父节点的子节点
    /// - 有 `parent_session_id` 但父 session 未恢复 → 降级为根节点，depth 重置为 0
    /// - 无 `parent_session_id` → 根节点
    fn build_spawn_tree(
        checkpoints: &mut HashMap<String, SessionCheckpoint>,
        recovered: &[String],
    ) -> (SpawnTree, Vec<String>) {
        let mut tree = SpawnTree::default();
        let mut demoted = Vec::new();
        let recovered_set: HashSet<&String> = recovered.iter().collect();

        for session_id in recovered {
            if let Some(cp) = checkpoints.get_mut(session_id) {
                match &cp.parent_session_id {
                    Some(parent_id) if recovered_set.contains(parent_id) => {
                        // 父 session 已恢复 — 注册为子节点
                        tree.children
                            .entry(parent_id.clone())
                            .or_default()
                            .push(session_id.clone());
                    }
                    Some(parent_id) => {
                        // 父 session 未恢复 — 降级为根节点，depth 重置为 0
                        tracing::info!(
                            session_id = %session_id,
                            parent_id = %parent_id,
                            "Session demoted to root: parent not recovered"
                        );
                        cp.depth = 0;
                        demoted.push(session_id.clone());
                        tree.roots.push(session_id.clone());
                    }
                    None => {
                        // 无父节点 — 确认为根节点
                        tree.roots.push(session_id.clone());
                    }
                }
            }
        }

        (tree, demoted)
    }

    /// Get the storage reference
    pub fn storage(&self) -> &S {
        &self.storage
    }
}

/// Spawn tree — tracks parent-child relationships between sessions.
///
/// Built during recovery from checkpoint data. Used by the Session module
/// to reconstruct the runtime spawn tree after gateway restart.
#[derive(Debug, Clone, Default)]
pub struct SpawnTree {
    /// parent_session_id → list of child session_ids
    pub children: HashMap<String, Vec<String>>,
    /// All root sessions (no parent or parent not recovered)
    pub roots: Vec<String>,
}

impl SpawnTree {
    /// Check if a session is a root node (no parent or parent not recovered).
    pub fn is_root(&self, session_id: &str) -> bool {
        self.roots.iter().any(|id| id == session_id)
    }

    /// Get children of a session.
    pub fn get_children(&self, session_id: &str) -> Option<&Vec<String>> {
        self.children.get(session_id)
    }

    /// Get all root session IDs.
    pub fn root_ids(&self) -> &[String] {
        &self.roots
    }

    /// Get the parent session ID of a given session.
    ///
    /// Returns `None` for root nodes or unknown sessions.
    pub fn get_parent(&self, session_id: &str) -> Option<&String> {
        if self.is_root(session_id) {
            return None;
        }
        self.children
            .iter()
            .find(|(_, children)| children.iter().any(|id| id == session_id))
            .map(|(parent, _)| parent)
    }
}

/// Extract the Tasks section ("## 开发步骤" or "## Tasks") from a plan file.
///
/// Returns `Some(content)` where `content` is the text between the Tasks
/// heading and the next `##` heading (or end of file), with leading/trailing
/// whitespace trimmed. Returns `None` if the file cannot be read or does not
/// contain a recognized Tasks heading.
pub fn extract_plan_tasks_section(plan_file_path: &str) -> Option<String> {
    let content = match std::fs::read_to_string(plan_file_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                plan_file = %plan_file_path,
                error = %e,
                "failed to read plan file for Tasks section extraction"
            );
            return None;
        }
    };

    extract_tasks_from_content(&content)
}

/// Extract the Tasks section from plan file content.
///
/// Looks for a heading line matching `## 开发步骤` or `## Tasks` (case-sensitive,
/// with optional leading `#` variants). Returns the content between that heading
/// and the next `##` heading or end of file.
pub(crate) fn extract_tasks_from_content(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut start = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed == "## 开发步骤" || trimmed == "## Tasks" {
            start = Some(i + 1);
            break;
        }
    }

    let start = start?;

    // Find the next ## heading after start
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(start) {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") || trimmed.starts_with("# ") {
            end = i;
            break;
        }
    }

    let section: String = lines[start..end].join("\n");
    let trimmed = section.trim().to_string();

    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

// ---------------------------------------------------------------------------
// Layer 4: ProgressTool call history fallback
// ---------------------------------------------------------------------------

/// Scan a list of [`SessionMessage`]s for ProgressTool calls and return
/// them as [`ProgressToolCallRecord`]s.
///
/// Scans all assistant messages for `ContentBlock::ToolUse` blocks where
/// `name == "Progress"`. The input arguments are parsed to extract
/// `step_index`, `status`, `summary`, and `error_message`.
///
/// Returns an empty `Vec` when no ProgressTool calls are found.
pub fn scan_progress_tool_calls(
    messages: &[crate::llm_session::SessionMessage],
) -> Vec<ProgressToolCallRecord> {
    use closeclaw_common::ContentBlock;

    let mut records = Vec::new();

    for msg in messages {
        if msg.role != "assistant" {
            continue;
        }
        for block in &msg.content_blocks {
            if let ContentBlock::ToolUse { name, input, .. } = block {
                if name != "Progress" {
                    continue;
                }
                if let Some(record) = parse_progress_call_record(input) {
                    records.push(record);
                }
            }
        }
    }

    records
}

/// Parse a ProgressTool call's input JSON into a [`ProgressToolCallRecord`].
///
/// Returns `None` if the input is not valid JSON or missing required fields.
pub(crate) fn parse_progress_call_record(input: &str) -> Option<ProgressToolCallRecord> {
    use closeclaw_common::ExecutionStepStatus;
    use serde_json::Value;

    let v: Value = serde_json::from_str(input).ok()?;

    let step_index = v.get("step_index")?.as_u64()? as usize;
    let status_str = v.get("status")?.as_str()?;

    let status = match status_str {
        "in_progress" => ExecutionStepStatus::InProgress,
        "completed" => ExecutionStepStatus::Completed,
        "failed" => ExecutionStepStatus::Failed,
        "skipped" => ExecutionStepStatus::Skipped,
        _ => return None,
    };

    let summary = v.get("summary").and_then(Value::as_str).map(String::from);
    let error_message = v
        .get("error_message")
        .and_then(Value::as_str)
        .map(String::from);

    Some(ProgressToolCallRecord {
        step_index,
        status,
        summary,
        error_message,
    })
}

/// Rebuild a [`PlanState`] from a list of [`ProgressToolCallRecord`]s.
///
/// Applies each record in order, skipping records that would violate the
/// step state machine. Returns the reconstructed `PlanState` with
/// `execution_steps` populated.
pub fn rebuild_plan_state_from_calls(
    calls: &[ProgressToolCallRecord],
) -> closeclaw_common::PlanState {
    use closeclaw_common::ExecutionStep;

    let mut plan_state = closeclaw_common::PlanState::new();

    if calls.is_empty() {
        return plan_state;
    }

    // Determine the maximum step index to size the steps vec
    let max_step = calls.iter().map(|c| c.step_index).max().unwrap_or(0);
    let total_steps = max_step + 1;

    // Initialize all steps as Pending
    plan_state.execution_steps = (0..total_steps)
        .map(|i| ExecutionStep {
            step_index: i,
            status: closeclaw_common::ExecutionStepStatus::Pending,
            summary: String::new(),
            error_message: None,
        })
        .collect();

    // Apply each call in order, ignoring invalid transitions
    for record in calls {
        let idx = record.step_index;
        if idx >= plan_state.execution_steps.len() {
            continue;
        }

        // Try the transition; skip if invalid (e.g., skipping steps)
        if plan_state.validate_transition(idx, &record.status).is_err() {
            continue;
        }

        plan_state.execution_steps[idx].status = record.status;
        if let Some(ref summary) = record.summary {
            plan_state.execution_steps[idx].summary = summary.clone();
        }
        if let Some(ref error) = record.error_message {
            plan_state.execution_steps[idx].error_message = Some(error.clone());
        }

        // Update current_step
        if matches!(
            record.status,
            closeclaw_common::ExecutionStepStatus::Completed
                | closeclaw_common::ExecutionStepStatus::Skipped
        ) {
            let next = idx + 1;
            if next < plan_state.execution_steps.len() {
                plan_state.current_step = Some(next);
            }
        } else if matches!(
            record.status,
            closeclaw_common::ExecutionStepStatus::InProgress
        ) {
            plan_state.current_step = Some(idx);
        }
    }

    plan_state
}

/// Rebuild a human-readable progress summary from ProgressTool call records.
///
/// Scans calls in reverse to find the latest status for each step,
/// then formats a summary suitable for injection into `system_appends`.
/// Returns an empty string when `calls` is empty.
pub fn rebuild_progress_summary_from_calls(calls: &[ProgressToolCallRecord]) -> String {
    if calls.is_empty() {
        return String::new();
    }

    let plan_state = rebuild_plan_state_from_calls(calls);
    plan_state.progress_summary()
}

// ---------------------------------------------------------------------------
// Layer 3: Approval tool call history fallback
// ---------------------------------------------------------------------------

/// Format approval tool call records into a human-readable summary.
///
/// Each record is rendered as a bullet point with tool name, plan summary,
/// and timestamp. Returns an empty string when `records` is empty.
pub fn format_approval_history(records: &[ApprovalToolCallRecord]) -> String {
    if records.is_empty() {
        return String::new();
    }

    let items: Vec<String> = records
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let ts = r
                .timestamp
                .map(|t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                .unwrap_or_else(|| "unknown".to_string());
            format!(
                "  {}. {} — {} (at {})",
                i + 1,
                r.tool_name,
                r.plan_summary,
                ts
            )
        })
        .collect();

    format!("[Approval History]\n{}", items.join("\n"))
}

// ---------------------------------------------------------------------------
// Layer 4: Message history / plan references fallback
// ---------------------------------------------------------------------------

/// Format plan references into a human-readable summary.
///
/// Each reference is rendered as a bullet point. Returns an empty string
/// when `references` is empty.
pub fn format_plan_references(references: &[String]) -> String {
    if references.is_empty() {
        return String::new();
    }

    let items: Vec<String> = references
        .iter()
        .enumerate()
        .map(|(i, r)| format!("  {}. {}", i + 1, r))
        .collect();

    format!("[Plan References]\n{}", items.join("\n"))
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "recovery_progress_tests.rs"]
mod recovery_progress_tests;
