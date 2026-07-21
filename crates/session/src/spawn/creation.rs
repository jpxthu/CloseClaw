//! Child session creation logic.
//!
//! Contains the core logic for creating a child `ConversationSession`
//! during a spawn operation. Handles workspace resolution, bootstrap mode
//! selection, system prompt construction, communication config setup,
//! task injection, and mode inheritance.
//!
//! This module is called by the gateway's `SessionManager::create_child_session`
//! wrapper, which handles the remaining registration steps (conversation_sessions
//! map, sessions map, children table, checkpoint persistence, timeout).

use std::path::PathBuf;
use std::sync::Arc;

use crate::bootstrap::loader::BootstrapMode;
use crate::llm_session::ChatSession;
use crate::llm_session::ConversationSession;
use crate::persistence::{PendingMessage, SessionMode};
use closeclaw_config::agents::ResolvedAgentConfig;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::communication::CommunicationConfig;
use super::context::SpawnCreationContext;
use super::types::SpawnMode;

/// Result of creating a child session's `ConversationSession`.
///
/// The gateway receives this result and performs the remaining registration
/// steps (inserting into maps, persisting checkpoint, tracking in spawn tree).
pub struct ChildSessionCreated {
    /// The newly created conversation session, ready for insertion.
    pub conversation_session: Arc<tokio::sync::RwLock<ConversationSession>>,
    /// The generated child session ID.
    pub session_id: String,
    /// Resolved workspace path for the child.
    pub workspace_path: PathBuf,
    /// Communication config applied to the child session.
    ///
    /// The gateway should use this value (not re-compute it) when persisting
    /// the checkpoint, to avoid duplication drift.
    pub communication_config: CommunicationConfig,
}

/// Parameters for creating a child `ConversationSession`.
///
/// Groups the per-call arguments that vary between spawn invocations,
/// keeping the function signature under the 6-parameter limit.
pub struct ChildSessionCreationParams<'a> {
    /// Parent session ID (the session that initiated the spawn).
    pub parent_session_id: &'a str,
    /// Parent agent ID (chat_id) for communication config.
    pub parent_agent_id: &'a str,
    /// Current spawn depth (0 = root).
    pub depth: u32,
    /// Task description to inject as the child's first message.
    pub task: &'a str,
    /// Whether to use minimal bootstrap mode.
    pub light_context: bool,
    /// Explicit workspace override (None = auto-resolve).
    pub workspace: Option<&'a str>,
    /// Spawn mode: Run (one-shot) or Session (persistent).
    pub mode: SpawnMode,
    /// Whether to fork (inherit parent conversation history).
    pub fork: bool,
    /// Explicit model override (highest priority).
    pub model_override: Option<&'a str>,
    /// Parent's subagents.model (second priority).
    pub parent_subagents_model: Option<&'a str>,
    /// Effective maximum spawn depth for the child.
    pub max_spawn_depth: u32,
    /// Optional prompt template text to inject into the child's system prompt
    /// tail (before the task message). Per design doc §9, the template goes
    /// into the system prompt, not the user message.
    pub prompt_template_prefix: Option<&'a str>,
}

/// Create a child `ConversationSession` for a spawned sub-agent.
///
/// Handles: workspace resolution, bootstrap mode, system prompt construction,
/// communication config, task injection, mode inheritance.
///
/// The caller (gateway) is responsible for:
/// - Registering in `conversation_sessions` and `sessions` maps
/// - Parent handle registration for cascade stop
/// - Checkpoint persistence
/// - Spawn tree registration
/// - Spawn timeout setup
pub async fn create_child_conversation_session(
    ctx: &dyn SpawnCreationContext,
    config: &ResolvedAgentConfig,
    params: &ChildSessionCreationParams<'_>,
) -> Result<ChildSessionCreated, String> {
    let child_session_id = Uuid::new_v4().to_string();
    let workdir_path =
        resolve_child_workspace(ctx, config, params.workspace, params.parent_session_id).await?;
    let model = resolve_model(params.model_override, params.parent_subagents_model, config);
    let bootstrap_mode = resolve_bootstrap_mode(params.light_context, config);
    let child_token = derive_child_token(ctx, params.parent_session_id).await?;

    let mut cs = ConversationSession::with_cancel_token(
        child_session_id.clone(),
        model,
        workdir_path.clone(),
        child_token,
    )
    .with_reasoning_level(ctx.default_reasoning_level())
    .with_bootstrap_mode(bootstrap_mode);

    wire_session_dependencies(ctx, &mut cs, config.hooks.clone());

    let spawn_context = build_spawn_context(
        params.depth,
        params.max_spawn_depth,
        params.parent_session_id,
        &params.mode,
        params.fork,
    );
    // Generate communication config: child may only communicate with parent.
    // Created here (not inside configure_spawn_behavior) so it can be returned
    // in ChildSessionCreated for checkpoint persistence by the gateway.
    let comm_config = CommunicationConfig::default_with_parent(Some(params.parent_agent_id));

    let behavior = SpawnBehaviorConfig {
        child_session_id: &child_session_id,
        agent_id: &config.id,
        bootstrap_mode,
        spawn_context,
    };
    cs = configure_spawn_behavior(ctx, cs, params, &behavior, &comm_config).await;

    let conversation_session = Arc::new(tokio::sync::RwLock::new(cs));

    Ok(ChildSessionCreated {
        conversation_session,
        session_id: child_session_id,
        workspace_path: workdir_path,
        communication_config: comm_config,
    })
}

/// Determine bootstrap mode from the `light_context` flag and agent config.
fn resolve_bootstrap_mode(light_context: bool, config: &ResolvedAgentConfig) -> BootstrapMode {
    if light_context {
        BootstrapMode::Minimal
    } else {
        config.bootstrap_mode
    }
}

/// Resolve the model to use via the priority chain:
/// explicit override > parent subagents.model > agent model > default.
fn resolve_model(
    model_override: Option<&str>,
    parent_subagents_model: Option<&str>,
    config: &ResolvedAgentConfig,
) -> String {
    model_override
        .map(String::from)
        .or(parent_subagents_model.map(String::from))
        .or(config.model.as_ref().map(|m| m.primary.clone()))
        .unwrap_or_else(|| "default".to_string())
}

/// Derive a child cancel token from the parent session's token tree.
async fn derive_child_token(
    ctx: &dyn SpawnCreationContext,
    parent_session_id: &str,
) -> Result<CancellationToken, String> {
    let parent_cs = ctx
        .get_parent_conversation_session(parent_session_id)
        .await
        .ok_or_else(|| {
            format!(
                "parent session not found in conversation_sessions: {}",
                parent_session_id
            )
        })?;
    let parent_guard = parent_cs.read().await;
    let token = parent_guard.child_cancel_token();
    drop(parent_guard);
    Ok(token)
}

/// Wire cross-cutting dependencies onto the session: shutdown handle,
/// LLM caller, system prompt builder, prompt overrides, and health hooks.
fn wire_session_dependencies(
    ctx: &dyn SpawnCreationContext,
    cs: &mut ConversationSession,
    agent_hooks: Vec<closeclaw_common::HookConfig>,
) {
    if let Some(signal) = ctx.shutdown_signal() {
        cs.set_shutdown_handle(signal);
    }
    if let Some(caller) = ctx.llm_caller() {
        cs.set_llm_caller(caller.clone());
        cs.init_health_checker(caller, agent_hooks);
    }
    if let Some(builder) = ctx.system_prompt_builder() {
        cs.set_system_prompt_builder(builder);
    }
    cs.set_prompt_overrides(ctx.prompt_overrides());
    // Inject dynamic prompt builder so child sessions can build
    // per-request dynamic layers (ChannelContext, SessionState, etc.).
    if let Some(dpb) = ctx.dynamic_prompt_builder() {
        cs.set_dynamic_prompt_builder(dpb);
    }
    // Inject skill listing provider for per-turn skill attachment.
    if let Some(provider) = ctx.skill_listing_provider() {
        cs.set_skill_listing_provider(provider);
    }
}

/// Per-call configuration for [`configure_spawn_behavior`].
///
/// Groups the arguments that vary per spawn invocation but are not part of
/// [`ChildSessionCreationParams`], keeping the function signature under
/// the 6-parameter limit.
struct SpawnBehaviorConfig<'a> {
    child_session_id: &'a str,
    agent_id: &'a str,
    bootstrap_mode: BootstrapMode,
    spawn_context: String,
}

/// Apply spawn-specific behavior to the session: system prompt, communication
/// config, fork history, task injection, and mode inheritance.
async fn configure_spawn_behavior(
    ctx: &dyn SpawnCreationContext,
    mut cs: ConversationSession,
    params: &ChildSessionCreationParams<'_>,
    behavior: &SpawnBehaviorConfig<'_>,
    comm_config: &CommunicationConfig,
) -> ConversationSession {
    // Build initial system prompt, then append spawn context.
    let base_prompt = cs
        .rebuild_system_prompt(
            behavior.child_session_id,
            behavior.agent_id,
            Some(behavior.bootstrap_mode),
        )
        .await;
    let mut system_prompt = format!("{}\n{}", base_prompt, behavior.spawn_context);
    // Inject prompt template into system prompt tail (design doc §9).
    // The template text goes into the system prompt, not the user message.
    if let Some(tpl_prefix) = params.prompt_template_prefix {
        system_prompt.push('\n');
        system_prompt.push_str(tpl_prefix);
    }
    cs.replace_system_prompt(system_prompt);

    // Mark as sub-agent so the sub-agent sparse prompt variant
    // is injected on subsequent LLM calls (design doc §5, §8).
    cs.set_sub_agent(true);

    // Apply communication config: child may only communicate with parent.
    cs = cs.with_communication_config(comm_config.clone());

    // Fork mode: inherit parent session's conversation history.
    if params.fork {
        if let Some(parent_cs) = ctx
            .get_parent_conversation_session(params.parent_session_id)
            .await
        {
            let parent_msgs = parent_cs.read().await.messages().to_vec();
            cs.clone_messages_from(&parent_msgs);
        }
    }

    // Inject task as pending message.
    let pending_msg = PendingMessage::with_role(
        format!("{}-task", behavior.child_session_id),
        params.task.to_string(),
        "user".to_string(),
    );
    cs.push_pending(pending_msg);

    // Inherit parent session mode (Plan Mode).
    if let Some(parent_cs) = ctx
        .get_parent_conversation_session(params.parent_session_id)
        .await
    {
        let parent_mode = parent_cs.read().await.session_mode();
        if parent_mode == SessionMode::Plan {
            cs.set_session_mode(SessionMode::Plan);
        }
    }

    cs
}

/// Resolve the workspace path for a child session.
///
/// Fallback order:
/// 1. Explicit `workspace` arg (if provided).
/// 2. `config.workspace` (if set).
/// 3. `<parent_workspace>/<child_agent_id>/<user_id>/` — subdirectory under the
///    parent session's workspace.
/// 4. `/tmp` (last resort).
async fn resolve_child_workspace(
    ctx: &dyn SpawnCreationContext,
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
    if let Some(parent_cs) = ctx.get_parent_conversation_session(parent_session_id).await {
        let parent_ws = {
            let guard = parent_cs.read().await;
            guard.workdir().to_path_buf()
        };
        let user_id = ctx
            .sender_id(parent_session_id)
            .await
            .unwrap_or_else(|| "default".to_string());
        let child_ws = parent_ws.join(&config.id).join(&user_id);
        std::fs::create_dir_all(&child_ws)
            .map_err(|e| format!("workspace creation failed: {}", e))?;
        return Ok(child_ws);
    }
    Ok(PathBuf::from("/tmp"))
}

/// Build the spawn context paragraph appended to child system prompts.
///
/// The paragraph tells the child agent:
/// - Its role (sub-agent)
/// - Current depth / maximum depth
/// - Communication behavior (push-based, no polling)
/// - Behavioral constraints (direct execution, no back-and-forth)
/// - Spawn guidance when depth allows further spawning
pub fn build_spawn_context(
    depth: u32,
    max_spawn_depth: u32,
    parent_session_id: &str,
    spawn_mode: &SpawnMode,
    fork: bool,
) -> String {
    let mode_str = match spawn_mode {
        SpawnMode::Run => "run",
        SpawnMode::Session => "session",
    };
    let mut ctx = format!(
        "## Spawn Context\n\
         You are running as a sub-agent.\n\
         - **parent_session_id**: {parent_session_id}\n\
         - **depth**: {depth} / **max_spawn_depth**: {max_spawn_depth}\n\
         - **spawn_mode**: {mode_str}\n\
         - **fork**: {fork}\n\
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

    ctx.push_str(&structured_output_guidance());

    ctx.push('\n');
    ctx
}

/// Structured output guidance paragraph (optional, per design doc).
fn structured_output_guidance() -> String {
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
     but it helps the parent agent process your results."
        .to_string()
}
