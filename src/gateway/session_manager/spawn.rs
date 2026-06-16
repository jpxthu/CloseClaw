//! Child session creation and tracking for `SessionManager`.
//!
//! Implements session-based spawn: `create_child_session` builds a
//! `ConversationSession` for the spawned sub-agent, registers it in
//! `sessions` / `conversation_sessions`, and tracks it in the `children`
//! table so `SpawnController` can enforce depth and concurrency limits.

use super::SessionManager;
use crate::agent::communication::CommunicationConfig;
use crate::config::agents::ResolvedAgentConfig;
use crate::gateway::Session;
use crate::llm::session::ChatSession;
use crate::llm::session::ConversationSession;
use crate::session::bootstrap::loader::{load_bootstrap_files, BootstrapMode};
use crate::session::persistence::PendingMessage;
use crate::system_prompt::builder::{build_from_workspace, WorkspaceBuildConfig};
use crate::system_prompt::workdir::build_workdir_context;
use crate::tools::ToolContext;
use std::path::PathBuf;
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

    /// Count active (non-completed) child sessions for a parent.
    pub async fn count_active_children(&self, parent_id: &str) -> usize {
        let children = self.children.read().await;
        children.get(parent_id).map(|v| v.len()).unwrap_or(0)
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
    /// 2. Resolve workspace path (3-level fallback: explicit arg → config → ensure subdir under self.workspace_dir).
    /// 3. Pick bootstrap mode (Minimal if light_context, else config's mode).
    /// 4. Build system prompt (mirrors `find_or_create` — load bootstrap files, build ToolContext, call `build_from_workspace`).
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
            },
        )
        .await;
        drop(tool_registry_guard);

        // 4a. Build spawn context — injected as the first part of
        //     the pending user message (not in the system prompt).
        //     The design doc requires the child's first message to
        //     contain context告知 (role, depth, comms) followed by
        //     the task content.
        let spawn_context =
            Self::build_spawn_context(parent_session_id, depth, max_spawn_depth, &mode, fork);

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

        // 6. Inject context告知 + task as pending message
        let content = format!("{}\n\n{}", spawn_context, task);
        let pending_msg = PendingMessage::new(format!("{}-task", child_session_id), content);
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
    pub(crate) fn build_spawn_context(
        parent_session_id: &str,
        depth: u32,
        remaining_depth: u32,
        spawn_mode: &SpawnMode,
        fork: bool,
    ) -> String {
        let mode_text = match spawn_mode {
            SpawnMode::Run => "run",
            SpawnMode::Session => "session",
        };
        let mut ctx = format!(
            "## Spawn Context\n\
             You are running as a sub-agent. \
             Parent session: {parent_session_id}. \
             Current depth: {depth}. \
             Remaining spawn depth: {remaining_depth}. \
             Spawn mode: {mode_text}. \
             Fork: {fork}.\n\
             **Communication behavior:** Your results are automatically \
             pushed back to the parent agent when you finish. \
             Do not poll for status. \
             If you need to wait for sub-agent results, use the yield \
             mechanism to end your current turn.\n\
             **Behavioral constraints:**\n\
             - Trust push-based completion notifications\n             - Do not call session query tools to check child agent status\n             - Execute the task directly; do not ask for confirmation \
               or suggest next steps — the parent agent handles that"
        );
        if remaining_depth > 0 {
            ctx.push_str(&format!(
                "\n\
             - You may spawn child agents for sub-tasks. \
               Your effective maximum depth for children is {remaining_depth}."
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

    /// Validate that a child session is owned by the given parent and
    /// was spawned in `Session` mode (persistent). Returns the child
    /// info on success, `None` otherwise.
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
            .and_then(|list| {
                list.iter()
                    .find(|info| info.session_id == child_id && info.mode == SpawnMode::Session)
            })
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
    pub(crate) async fn kill_child(&self, parent_id: &str, child_id: &str) -> Result<(), String> {
        let cs = self
            .get_conversation_session(child_id)
            .await
            .ok_or_else(|| format!("child session not found: {}", child_id))?;

        // Cascade-stop: cancels the token tree and cleans up tool
        // handles / child handles.
        cs.read().await.stop(true).await;

        // Remove from conversation_sessions.
        {
            let mut conv_sessions = self.conversation_sessions.write().await;
            conv_sessions.remove(child_id);
        }

        // Unregister child handle from parent's ConversationSession.
        {
            let conv_sessions = self.conversation_sessions.read().await;
            if let Some(parent_cs) = conv_sessions.get(parent_id) {
                parent_cs.read().await.unregister_child_handle(child_id);
            }
        }

        // Remove from sessions.
        {
            let mut sessions = self.sessions.write().await;
            sessions.remove(child_id);
        }

        // Remove from children tracking table.
        {
            let mut children = self.children.write().await;
            if let Some(list) = children.get_mut(parent_id) {
                list.retain(|info| info.session_id != child_id);
                if list.is_empty() {
                    children.remove(parent_id);
                }
            }
        }

        Ok(())
    }
}
