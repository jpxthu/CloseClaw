//! Child session creation and tracking for `SessionManager`.
//!
//! Implements session-based spawn: `create_child_session` builds a
//! `ConversationSession` for the spawned sub-agent, registers it in
//! `sessions` / `conversation_sessions`, and tracks it in the `children`
//! table so `SpawnController` can enforce depth and concurrency limits.

use super::SessionManager;
use crate::config::agents::ResolvedAgentConfig;
use crate::gateway::session_manager::communication::CommunicationConfig;
use crate::gateway::Session;
use crate::llm::session::ChatSession;
use crate::llm::session::ConversationSession;
use crate::session::bootstrap::loader::{load_bootstrap_files, BootstrapMode};
use crate::session::persistence::{
    PendingMessage, PersistenceError, SessionCheckpoint, SessionStatus,
};
use crate::system_prompt::builder::{build_from_workspace, WorkspaceBuildConfig};
use crate::system_prompt::workdir::build_workdir_context;
use crate::tools::ToolContext;
use std::collections::HashSet;
use std::path::PathBuf;
use tracing::warn;
use uuid::Uuid;

/// Metadata for a child session tracked by the parent.
#[derive(Debug, Clone)]
pub struct ChildSessionInfo {
    pub session_id: String,
    pub parent_session_id: String,
    pub agent_id: String,
    pub depth: u32,
    pub mode: SpawnMode,
}

/// Spawn mode for child sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnMode {
    /// One-shot: child runs one LLM turn then completes.
    Run,
    /// Persistent: child stays alive for subsequent steering.
    Session,
}

impl SessionManager {
    /// Get the depth of a session. Returns None if session does not exist.
    pub async fn get_session_depth(&self, session_id: &str) -> Option<u32> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(|s| s.depth)
    }

    /// Get the effective max spawn depth budget for a session.
    ///
    /// Reads from the session's checkpoint (persisted by `create_child_session`).
    /// Returns `None` if the session has no stored effective budget (e.g. root
    /// sessions created before Step 1.1, or when storage is unavailable).
    pub async fn get_effective_max_spawn_depth(&self, session_id: &str) -> Option<u32> {
        let storage = self.storage.read().await;
        let storage = storage.as_ref()?;
        match storage.load_checkpoint(session_id).await {
            Ok(Some(cp)) => cp.effective_max_spawn_depth,
            _ => None,
        }
    }

    /// Count active (non-completed) child sessions for a parent.
    pub async fn count_active_children(&self, parent_id: &str) -> usize {
        let children = self.children.read().await;
        children.get(parent_id).map(|v| v.len()).unwrap_or(0)
    }

    /// List all active child session IDs for a parent.
    ///
    /// Returns a cloned snapshot so the caller does not hold the
    /// `children` lock across await points.
    #[allow(dead_code)] // Used in spawn_budget_tests.rs for cascade simulation
    pub(crate) async fn list_active_child_ids(&self, parent_id: &str) -> Vec<String> {
        let children = self.children.read().await;
        children
            .get(parent_id)
            .map(|list| list.iter().map(|info| info.session_id.clone()).collect())
            .unwrap_or_default()
    }

    /// Register a child session under its parent. Called after child session creation.
    pub(crate) async fn register_child(&self, parent_id: &str, info: ChildSessionInfo) {
        let mut children = self.children.write().await;
        children
            .entry(parent_id.to_string())
            .or_default()
            .push(info);
    }

    /// Create a child session for a spawned sub-agent.
    ///
    /// Returns the new child session_id on success.
    ///
    /// Workflow:
    /// 1. Generate a new UUID-based session id.
    /// 2. Resolve workspace path (3-level fallback: explicit
    ///    arg → config → ensure subdir under parent workspace).
    /// 3. Pick bootstrap mode (Minimal if light_context, else config's mode).
    /// 4. Build system prompt (mirrors `find_or_create` —
    ///    load bootstrap files, build ToolContext, call
    ///    `build_from_workspace`).
    /// 5. Construct `ConversationSession` (with system prompt + default reasoning level).
    /// 6. Push `task` as the first pending message so the child picks it up.
    /// 7. Register the new child in `conversation_sessions` + `sessions` + `children` tables.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_child_session(
        &self,
        config: &ResolvedAgentConfig,
        parent_session_id: &str,
        depth: u32,
        task: &str,
        light_context: bool,
        workspace: Option<&str>,
        mode: SpawnMode,
        fork: bool,
        allowed_tools: Option<Vec<String>>,
        model_override: Option<&str>,
        parent_subagents_model: Option<&str>,
        max_spawn_depth: u32,
    ) -> Result<String, String> {
        // ── Increment busy count for drain tracking ────────────────────
        if let Some(sh) = self.get_shutdown_handle().await {
            sh.increment_busy();
        }

        // ── Shutdown gate: reject new child session creation ──────────
        if let Some(sh) = self.get_shutdown_handle().await {
            if sh.is_shutting_down() {
                tracing::warn!(
                    parent_session_id = %parent_session_id,
                    "rejecting child session creation: daemon is shutting down"
                );
                if let Some(sh) = self.get_shutdown_handle().await {
                    sh.decrement_busy();
                }
                return Err("daemon is shutting down".into());
            }
        }

        // Apply tool whitelist override: when allowed_tools is provided,
        // replace the config's tools list so the child session only has
        // access to the specified tools.
        let config = if let Some(ref tools) = allowed_tools {
            let mut overridden = config.clone();
            overridden.tools = tools.clone();
            overridden
        } else {
            config.clone()
        };
        let config = &config;

        // 1. Generate child session_id
        let child_session_id = Uuid::new_v4().to_string();

        // 2. Determine workspace path (3-level fallback)
        let workdir_path = self
            .resolve_child_workspace(config, workspace, parent_session_id)
            .await?;

        // 3. Determine bootstrap_mode
        let bootstrap_mode = if light_context {
            BootstrapMode::Minimal
        } else {
            config.bootstrap_mode
        };

        // 4. Build system prompt (mirror find_or_create)
        let bootstrap_files = if let Some(ref workspace_root) = self.workspace_dir {
            load_bootstrap_files(workspace_root, bootstrap_mode)
                .unwrap_or_default()
                .into_iter()
                .collect()
        } else {
            vec![]
        };
        let tool_registry_guard = self.tool_registry.read().await;
        let tool_registry_ref = tool_registry_guard.as_ref().map(|r| r.as_ref());
        let skill_registry = self.skill_registry.read().await.clone();
        let agent_registry = self.agent_registry.read().await.clone();
        let agent_id = config.id.clone();
        let tool_ctx = ToolContext {
            agent_id: agent_id.clone(),
            workdir: Some(build_workdir_context(&workdir_path.to_string_lossy())),
            session_id: Some(child_session_id.clone()),
            call_id: None,
            session: None,
        };
        let workspace_root = self.workspace_dir.clone().unwrap_or_default();
        // Pass agent-level tool filtering from the resolved config.
        let filters = Self::extract_agent_filters(config);
        let prompt = build_from_workspace(
            &workspace_root,
            WorkspaceBuildConfig {
                bootstrap_files,
                tool_registry: tool_registry_ref,
                tool_ctx: &tool_ctx,
                skill_registry,
                agent_id: Some(&agent_id),
                agent_tools: filters.agent_tools,
                agent_disallowed_tools: filters.agent_disallowed_tools,
                agent_skills: filters.agent_skills,
                dynamic_sections: vec![],
                append_section: None,
                agent_registry,
            },
        )
        .await;
        drop(tool_registry_guard);

        // 4a. Append spawn context to the system prompt so the child
        //     agent knows its role, depth limits, and communication
        //     behavior.  Non-spawn sessions never reach this path.
        let spawn_context = Self::build_spawn_context(depth, max_spawn_depth);
        let prompt = format!("{}\n{}", prompt, spawn_context);

        // 5. Create ConversationSession
        // Model priority: explicit model param > parent agent.subagents.model
        // > target agent.model > system default
        let model = model_override
            .map(String::from)
            .or(parent_subagents_model.map(String::from))
            .or(config.model.clone())
            .unwrap_or_else(|| "default".to_string());

        // 5a. Wire child into parent's cancel-token tree (Step 1.5).
        // Look up the parent session's ConversationSession so we can
        // derive a child token from its cancellation source. The
        // token tree is one-way: parent.cancel() cascades to this
        // child automatically; a child cancel() never affects the
        // parent.
        let child_token = {
            let conv_sessions = self.conversation_sessions.read().await;
            let parent_cs = conv_sessions.get(parent_session_id).ok_or_else(|| {
                format!(
                    "parent session not found in conversation_sessions: {}",
                    parent_session_id
                )
            })?;
            // Bind the read guard to a local so it is dropped before
            // `conv_sessions` goes out of scope at the end of this
            // block. The CancellationToken returned by
            // `child_cancel_token` is owned (it's a fresh child of
            // the parent's token tree), so once the guard is dropped
            // we can still use it.
            let parent_guard = parent_cs.read().await;
            let token = parent_guard.child_cancel_token();
            drop(parent_guard);
            token
        };

        let mut cs = ConversationSession::with_cancel_token(
            child_session_id.clone(),
            model,
            workdir_path.clone(),
            child_token,
        )
        .with_system_prompt(prompt)
        .with_reasoning_level(self.default_reasoning_level);

        // Wire shutdown handle for busy-count tracking.
        if let Some(sh) = self.get_shutdown_handle().await {
            cs.set_shutdown_handle(sh);
        }

        // 5b. Generate communication config: child may only
        //     communicate with its parent agent.
        let parent_agent_id = self.get_chat_id(parent_session_id).await;
        let comm_config = CommunicationConfig::default_with_parent(parent_agent_id.as_deref());
        cs = cs.with_communication_config(comm_config);

        // 6a. Fork mode: inject parent session's conversation history
        //     before the task so the child inherits the parent's context.
        if fork {
            if let Some(parent_cs) = self.get_conversation_session(parent_session_id).await {
                let parent_msgs = parent_cs.read().await.messages().to_vec();
                cs.clone_messages_from(&parent_msgs);
            }
        }

        // 6. Inject task as pending message
        let pending_msg =
            PendingMessage::new(format!("{}-task", child_session_id), task.to_string());
        cs.push_pending(pending_msg);

        // 7. Register to conversation_sessions and sessions mappings
        let child_cs_arc = std::sync::Arc::new(tokio::sync::RwLock::new(cs));
        {
            let mut conv_sessions = self.conversation_sessions.write().await;
            conv_sessions.insert(child_session_id.clone(), child_cs_arc.clone());
        }

        // 7a. Register the child session handle with the parent so
        // stop(cascade=true) can recursively stop this child (Step 1.5).
        // We re-borrow `conversation_sessions` rather than holding the
        // parent's Arc here to avoid aliasing the same write lock
        // through both arms; a fresh read is sufficient and cheap.
        {
            let conv_sessions = self.conversation_sessions.read().await;
            if let Some(parent_cs) = conv_sessions.get(parent_session_id) {
                parent_cs.read().await.register_child_handle(
                    &child_session_id,
                    std::sync::Arc::downgrade(&child_cs_arc),
                );
            }
            // If the parent is missing we already inserted the child
            // into conversation_sessions; the announce path will
            // surface the orphan via cleanup. We deliberately do not
            // roll back here to keep the error path simple — the
            // child is still reachable and completable.
        }
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(
                child_session_id.clone(),
                Session {
                    id: child_session_id.clone(),
                    agent_id: config.id.clone(),
                    channel: "spawn".to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                    depth,
                },
            );
        }

        // 7b. Persist checkpoint with parent_session_id and depth so
        //     flush_all / recovery can reconstruct the spawn tree.
        let cp = SessionCheckpoint::new(child_session_id.clone())
            .with_status(SessionStatus::Active)
            .with_platform("spawn".to_string())
            .with_agent_id(config.id.clone())
            .with_parent_session_id(parent_session_id.to_string())
            .with_depth(depth)
            .with_effective_max_spawn_depth(Some(max_spawn_depth));
        if let Some(storage) = self.storage.read().await.as_ref() {
            if let Err(e) = storage.save_checkpoint(&cp).await {
                warn!(
                    session_id = %child_session_id,
                    error = %e,
                    "failed to save child session checkpoint"
                );
            }
        }

        // 8. Register to children tracking table
        self.register_child(
            parent_session_id,
            ChildSessionInfo {
                session_id: child_session_id.clone(),
                parent_session_id: parent_session_id.to_string(),
                agent_id: config.id.clone(),
                depth,
                mode,
            },
        )
        .await;

        // 9. Return child session id
        Ok(child_session_id)
    }

    /// Build the spawn context paragraph appended to child system prompts.
    ///
    /// The paragraph tells the child agent:
    /// - Its role (sub-agent)
    /// - Current depth / maximum depth
    /// - Communication behavior (push-based, no polling)
    /// - Behavioral constraints (direct execution, no back-and-forth)
    /// - Spawn guidance when depth allows further spawning
    pub(crate) fn build_spawn_context(depth: u32, max_spawn_depth: u32) -> String {
        let mut ctx = format!(
            "## Spawn Context\n\
             You are running as a sub-agent. \
             Current depth: {depth} / Maximum depth: {max_spawn_depth}.\n\
             **Communication behavior:** Your results are automatically \
             pushed back to the parent agent when you finish. \
             Do not poll for status. \
             If you need to wait for sub-agent results, use the yield \
             mechanism to end your current turn.\n\
             **Behavioral constraints:**\n\
             - Trust push-based completion
               notifications\n             - Do not call session query tools
               to check child agent status\n             - Execute the task directly;
               do not ask for confirmation \
               or suggest next steps — the parent agent handles that"
        );
        if depth < max_spawn_depth {
            let upper = max_spawn_depth - depth;
            ctx.push_str(&format!(
                "\n\
             - You may spawn child agents for sub-tasks. \
               Your effective maximum depth for children is {upper}."
            ));
        }

        // Structured output guidance (optional, per design doc §结构化输出).
        // Helps the parent agent parse and act on the child's results.
        ctx.push_str(
            "\n\
             **Structured output (optional):** \
             When you complete your task, consider structuring your \
             response with the following sections:\n\
             - **Task scope**: one-sentence confirmation of what you \
               understood\n\
             - **Execution results**: key findings or answers\n\
             - **Files involved**: relevant file paths\n\
             - **File changes**: modified files and what changed\n\
             - **Issues found**: problems or risks encountered\n\
             Structured output is a suggestion — you may reply freely — \
             but it helps the parent agent process your results.",
        );

        ctx.push('\n');
        ctx
    }

    /// Resolve the workspace path for a child session.
    ///
    /// Fallback order:
    /// 1. Explicit `workspace` arg (if provided) — used as-is.
    /// 2. `config.workspace` (if set).
    /// 3. `<parent_workspace>/<child_agent_id>/<user_id>/` — subdirectory under the
    ///    parent session's workspace, using the actual user_id from the parent's
    ///    session context (fallback to "default" if unavailable).
    /// 4. `/tmp` (last resort).
    async fn resolve_child_workspace(
        &self,
        config: &ResolvedAgentConfig,
        workspace: Option<&str>,
        parent_session_id: &str,
    ) -> Result<PathBuf, String> {
        if let Some(ws) = workspace {
            return Ok(PathBuf::from(ws));
        }
        if let Some(ref ws) = config.workspace {
            return Ok(ws.clone());
        }
        // Level 3: create subdirectory under parent session's workspace.
        let parent_workspace = {
            let conv_sessions = self.conversation_sessions.read().await;
            conv_sessions.get(parent_session_id).map(|cs| {
                // Clone the path while holding a short-lived read lock;
                // the guard is dropped when the closure returns.
                let cs_clone = cs.clone();
                async move {
                    let guard = cs_clone.read().await;
                    guard.workdir().to_path_buf()
                }
            })
        };
        if let Some(make_parent_ws) = parent_workspace {
            let parent_ws = make_parent_ws.await;
            // Use actual user_id from parent session context instead of
            // hardcoding "default", per design doc: workspace path =
            // <parent_workspace>/<child_agent_id>/<user_id>/.
            let user_id = self
                .get_sender_id(parent_session_id)
                .await
                .unwrap_or_else(|| "default".to_string());
            let child_ws = parent_ws.join(&config.id).join(&user_id);
            std::fs::create_dir_all(&child_ws)
                .map_err(|e| format!("workspace creation failed: {}", e))?;
            return Ok(child_ws);
        }
        Ok(PathBuf::from("/tmp"))
    }

    /// Validate that a child session is owned by the given parent.
    /// Returns the child info on success, `None` otherwise.
    ///
    /// Pure read operation — does not hold the children lock across
    /// any await point.
    pub(crate) async fn validate_child_ownership(
        &self,
        parent_id: &str,
        child_id: &str,
    ) -> Option<ChildSessionInfo> {
        let children = self.children.read().await;
        children
            .get(parent_id)
            .and_then(|list| list.iter().find(|info| info.session_id == child_id))
            .cloned()
    }

    /// Inject a new task into a persistent child session's pending
    /// message queue. The task is enqueued (FIFO) and will be
    /// consumed after the child's current turn completes.
    pub(crate) async fn steer_child(&self, child_id: &str, task: &str) -> Result<(), String> {
        let cs = self
            .get_conversation_session(child_id)
            .await
            .ok_or_else(|| format!("child session not found: {}", child_id))?;
        let pending_msg = PendingMessage::new(format!("{}-steer", child_id), task.to_string());
        cs.write().await.push_pending(pending_msg);
        Ok(())
    }

    /// Force-terminate a child session: cancel its token tree,
    /// remove it from `conversation_sessions`, `sessions`, and
    /// the parent's `children` tracking table. The archive is
    /// preserved (no purge).
    ///
    /// All descendants are also cleaned up recursively (per design
    /// doc §级联 Kill — from deepest to shallowest). The `children`
    /// table is traversed via BFS to discover all descendant session
    /// IDs before any removals, preventing lock-ordering issues.
    pub(crate) async fn kill_child(&self, parent_id: &str, child_id: &str) -> Result<(), String> {
        // 1. Recursively collect all descendant session IDs via
        //    BFS on the `children` table.
        let descendant_ids = self.collect_descendant_ids(child_id).await;

        // 2. Get the child's conversation session, verify it exists,
        //    and cascade-stop its token tree.
        if let Some(cs) = self.get_conversation_session(child_id).await {
            cs.read().await.stop(true).await;
        } else {
            return Err(format!("child session not found: {}", child_id));
        }

        // 3. Remove child + descendants from conversation_sessions.
        {
            let mut cs = self.conversation_sessions.write().await;
            cs.remove(child_id);
            for id in &descendant_ids {
                cs.remove(id);
            }
        }

        // 4. Unregister child handle from parent.
        {
            let cs = self.conversation_sessions.read().await;
            if let Some(parent_cs) = cs.get(parent_id) {
                parent_cs.read().await.unregister_child_handle(child_id);
            }
        }

        // 5. Remove child + descendants from sessions.
        {
            let mut sessions = self.sessions.write().await;
            sessions.remove(child_id);
            for id in &descendant_ids {
                sessions.remove(id);
            }
        }

        // 6. Remove child + descendants from children table.
        self.remove_children_entries(parent_id, child_id, &descendant_ids)
            .await;

        Ok(())
    }

    /// Cascade-kill all active children of a session.
    ///
    /// Called when a parent session ends (via `/new`) or is archived
    /// by the sweeper, per design doc §生命周期联动.
    /// Iterates direct children and calls `kill_child` for each,
    /// which recursively handles deeper descendants.
    pub(crate) async fn cascade_kill_all_children(&self, parent_id: &str) {
        // Snapshot child IDs to avoid holding the lock across kill calls.
        let child_ids: Vec<String> = {
            let children = self.children.read().await;
            children
                .get(parent_id)
                .map(|list| list.iter().map(|i| i.session_id.clone()).collect())
                .unwrap_or_default()
        };
        for child_id in child_ids {
            if let Err(e) = self.kill_child(parent_id, &child_id).await {
                tracing::warn!(
                    parent = %parent_id,
                    child = %child_id,
                    error = %e,
                    "cascade_kill_all_children: failed to kill child"
                );
            }
        }
    }

    /// Remove a direct child and all its descendants from the
    /// `children` tracking table.
    ///
    /// Handles: (a) removing the direct child from `parent_id`'s
    /// list, (b) removing each descendant from its own parent's
    /// list, and (c) removing any entries where a descendant is
    /// itself a parent of further descendants.
    async fn remove_children_entries(
        &self,
        parent_id: &str,
        child_id: &str,
        descendant_ids: &[String],
    ) {
        let mut children = self.children.write().await;

        // Remove the direct child from parent's children list.
        if let Some(list) = children.get_mut(parent_id) {
            list.retain(|info| info.session_id != child_id);
            if list.is_empty() {
                children.remove(parent_id);
            }
        }

        // Remove each descendant from its own parent's list
        // and clean up any sub-entries.
        for id in descendant_ids {
            let parent = children
                .values_mut()
                .find(|list| list.iter().any(|info| info.session_id == *id));
            if let Some(list) = parent {
                list.retain(|info| info.session_id != *id);
            }
            children.remove(id);
        }
    }

    /// Collect all descendant session IDs of a given session via BFS
    /// on the `children` table.
    ///
    /// Returns session IDs in reverse BFS order (deepest first,
    /// shallowest last) so that the caller removes leaves before
    /// their parents — matching the design doc requirement to
    /// "terminate from deepest to shallowest".
    async fn collect_descendant_ids(&self, session_id: &str) -> Vec<String> {
        let children = self.children.read().await;
        let mut result = Vec::new();
        let mut queue = std::collections::VecDeque::new();

        // Seed the queue with the direct children of session_id.
        if let Some(list) = children.get(session_id) {
            for info in list {
                queue.push_back(info.session_id.clone());
            }
        }

        while let Some(current) = queue.pop_front() {
            result.push(current.clone());
            // Enqueue this session's own children (grandchildren).
            if let Some(list) = children.get(&current) {
                for info in list {
                    queue.push_back(info.session_id.clone());
                }
            }
        }

        // Reverse so deepest descendants come first.
        result.reverse();
        result
    }

    /// Rebuild the spawn tree (children table) from persisted checkpoints.
    ///
    /// Called at startup after all sessions are restored. Iterates all
    /// active + archived checkpoints and registers parent-child
    /// relationships. Sessions whose parent has been swept are
    /// silently skipped (degraded to root).
    pub async fn rebuild_spawn_tree(&self) -> Result<(), PersistenceError> {
        let storage_arc = {
            let guard = self.storage.read().await;
            match guard.as_ref() {
                Some(s) => std::sync::Arc::clone(s),
                None => return Ok(()),
            }
        };

        // Collect all session_ids from active + archived.
        let mut all_ids: Vec<String> = {
            let active = storage_arc.list_active_sessions().await?;
            let archived = storage_arc.list_archived_sessions().await?;
            let mut ids = active;
            ids.extend(archived);
            ids
        };
        all_ids.sort();
        all_ids.dedup();

        let known_ids: HashSet<&str> = all_ids.iter().map(|s| s.as_str()).collect();
        let mut rebuilt: u32 = 0;
        let mut orphan_ids: Vec<String> = Vec::new();

        for session_id in &all_ids {
            let cp = match storage_arc.load_checkpoint(session_id).await {
                Ok(Some(cp)) => cp,
                Ok(None) => {
                    warn!(
                        session_id = %session_id,
                        "checkpoint returned None during spawn_tree rebuild, skipping"
                    );
                    continue;
                }
                Err(e) => {
                    warn!(
                        session_id = %session_id,
                        error = %e,
                        "failed to load checkpoint during spawn_tree rebuild, skipping"
                    );
                    continue;
                }
            };
            let parent_id = match cp.parent_session_id.as_deref() {
                Some(p) => p,
                None => continue, // root node
            };
            if !known_ids.contains(parent_id) {
                // Parent missing — orphan session. Collect for batch
                // depth reset after the loop to avoid acquiring the
                // write lock per iteration.
                orphan_ids.push(session_id.clone());
                continue;
            }
            self.register_child(
                parent_id,
                ChildSessionInfo {
                    session_id: session_id.clone(),
                    parent_session_id: parent_id.to_string(),
                    agent_id: cp.agent_id.unwrap_or_default(),
                    depth: cp.depth,
                    // All restored child sessions are treated as
                    // persistent (Session) because run-mode sessions
                    // that completed before shutdown are not expected
                    // to appear in the checkpoint list.
                    mode: SpawnMode::Session,
                },
            )
            .await;
            rebuilt += 1;
        }

        // Batch-reset orphan session depths to 0 (degrade to root).
        // Single write lock acquisition instead of per-orphan lock.
        if !orphan_ids.is_empty() {
            let mut sessions = self.sessions.write().await;
            for orphan_id in &orphan_ids {
                if let Some(s) = sessions.get_mut(orphan_id) {
                    s.depth = 0;
                }
            }
        }

        tracing::info!(rebuilt, "spawn_tree rebuilt from checkpoints");
        Ok(())
    }
}
