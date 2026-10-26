//! Daemon struct definitions, extracted from mod.rs to stay under the
//! 1000-line limit imposed by CONTRIBUTING.md.

use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SpawnController;
use closeclaw_gateway::{Gateway, SessionManager};
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::LLMRegistry;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::PermissionEngine;
use closeclaw_session::storage::SqliteStorage;
use closeclaw_skills::{BuiltinSkillRegistry, DiskSkillRegistry};
use closeclaw_system_prompt::sections::SectionCache;
use closeclaw_tools::builtin::PlanExecConfirmFlow;
use closeclaw_tools::ToolRegistry;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::watch;

use crate::config_watcher;
use crate::gateway_restart::RestartHandle;

/// Global daemon state
pub struct Daemon {
    /// Gateway instance — wrapped in Mutex for restart-time swap.
    /// Read via `self.gateway()` helper; write during restart only.
    pub gateway: Arc<tokio::sync::Mutex<Arc<Gateway>>>,
    /// Chat RPC server task handle — wrapped for restart-time swap.
    #[allow(dead_code)]
    pub(crate) chat_handle: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub agent_registry: Arc<closeclaw_agent::registry::AgentRegistry>,
    pub permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    pub shutdown: Arc<crate::shutdown::ShutdownHandle>,
    /// Session manager for session lifecycle management
    pub session_manager: Arc<SessionManager>,
    /// SQLite storage for session persistence
    pub storage: Arc<SqliteStorage>,
    /// Shutdown sender for ArchiveSweeper
    pub sweeper_shutdown_tx: watch::Sender<()>,
    /// Shutdown sender for AnnounceSweeper
    pub announce_shutdown_tx: watch::Sender<()>,
    /// Shutdown sender for DreamingScheduler
    pub dreaming_scheduler_shutdown_tx: watch::Sender<()>,

    /// Shared skill registry, rebuilt on demand via admin RPC
    pub skill_registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
    /// Builtin skill registry — compiled-in skills, not subject to rescan
    pub builtin_skill_registry: Arc<BuiltinSkillRegistry>,
    /// Slash command handler registry — shared with SlashDispatcher;
    /// allows late registration of SkillSlashHandler after registries are ready.
    pub slash_registry: Arc<closeclaw_slash::registry::HandlerRegistry>,
    /// Config file watcher handle (RAII: stops on drop)
    pub(crate) _config_watcher: Option<config_watcher::ConfigWatcherHandle>,
    /// ConfigWatcher subscriber task handle — joined in Phase 3
    /// to confirm all 5 background tasks have stopped.
    pub(crate) config_watcher_subscriber_handle: Option<tokio::task::JoinHandle<()>>,
    /// Daemon-level approval orchestrator
    pub approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    /// Plan execution confirmation flow (independent from permission approval)
    pub confirm_flow: Arc<PlanExecConfirmFlow>,
    /// Admin RPC server task handle (drop cancels the task)
    #[allow(dead_code)]
    pub(crate) admin_handle: Option<tokio::task::JoinHandle<()>>,
    /// Path to the admin RPC socket file (cleaned up on shutdown)
    pub(crate) admin_socket_path: PathBuf,

    /// Path to the chat RPC socket file (cleaned up on shutdown)
    pub(crate) chat_socket_path: PathBuf,
    /// Join handle for ArchiveSweeper background task
    pub(crate) archive_sweeper_handle: Option<tokio::task::JoinHandle<()>>,
    /// Join handle for AnnounceSweeper background task
    pub(crate) announce_sweeper_handle: Option<tokio::task::JoinHandle<()>>,
    /// Join handle for DreamingScheduler background task
    pub(crate) dreaming_scheduler_handle: Option<tokio::task::JoinHandle<()>>,
    /// Shutdown sender for PlanArchiveSweeper
    pub(crate) plan_archive_shutdown_tx: watch::Sender<()>,
    /// Join handle for PlanArchiveSweeper background task
    pub(crate) plan_archive_sweeper_handle: Option<tokio::task::JoinHandle<()>>,
    /// Media cleanup task handle — shuts down the periodic cleanup loop.
    pub(crate) media_cleanup_handle: Option<closeclaw_tasks::media_cleanup::MediaCleanupHandle>,
    /// ConfigManager reference — used to read shutdown timeouts at runtime.
    pub config_manager: Arc<closeclaw_config::ConfigManager>,
    /// SpawnController reference — manages sub-agent lifecycle
    pub spawn_controller: Option<Arc<SpawnController>>,
    /// SystemPromptBuilder reference — static-layer prompt construction
    pub system_prompt_builder: Option<Arc<dyn closeclaw_common::SystemPromptBuilder>>,
    /// LLM provider registry — reads models.json, constructs LLM clients.
    /// Initialized in Phase 2, consumed by LLM call chain assembly.
    pub llm_registry: Arc<LLMRegistry>,
    /// Unified fallback client built in layer 2 from registered providers.
    /// Shared across all LLM call sites (SessionManager, active searcher,
    /// compaction, gateway restart).
    pub fallback_client: Arc<UnifiedFallbackClient>,
    /// Gateway restart state machine — tracks Pending/Executing transitions.
    pub(crate) restart_state: RestartHandle,
    /// Receiver for restart-class config change signals from the config watcher.
    /// Processes incoming signals and calls `request_gateway_restart()`.
    /// Wrapped in `Option` so it can be taken in `run()` for the restart loop.
    pub(crate) restart_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    /// Receiver for admin RPC restart commands (force=true, cancel=false).
    /// Processed in `run()` select loop alongside config watcher signals.
    pub(crate) admin_restart_rx: Option<tokio::sync::mpsc::Receiver<bool>>,
    /// Path to the PID file — written on start, cleaned up on exit.
    pub(crate) pid_file_path: PathBuf,
}

impl Daemon {
    /// Get a clone of the current Gateway Arc.
    ///
    /// This is the preferred read path — it briefly locks the Mutex,
    /// clones the inner Arc, and releases the lock immediately.
    pub async fn gateway(&self) -> Arc<Gateway> {
        self.gateway.lock().await.clone()
    }

    /// Replace the Gateway instance (used during restart).
    pub async fn set_gateway(&self, gw: Arc<Gateway>) {
        *self.gateway.lock().await = gw;
    }

    /// Take the chat RPC JoinHandle (used during restart).
    pub async fn take_chat_handle(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.chat_handle.lock().await.take()
    }

    /// Store a new chat RPC JoinHandle (used during restart).
    pub async fn set_chat_handle(&self, h: tokio::task::JoinHandle<()>) {
        *self.chat_handle.lock().await = Some(h);
    }

    /// Take the ready-receiver for the restart watchdog channel.
    /// Returns `None` if already taken.
    pub fn take_restart_ready_rx(&self) -> Option<tokio::sync::mpsc::Receiver<Vec<String>>> {
        self.restart_state.take_ready_rx()
    }
}

/// Dependencies for Phase 5 background initialization.
///
/// Bundles external references that `init_phase_5_background` needs
/// from earlier phases, keeping the function signature within the 6-parameter
/// limit imposed by CONTRIBUTING.md.
pub(crate) struct Phase5Deps<'a> {
    pub config_manager: &'a Arc<ConfigManager>,
    pub agent_registry: &'a Arc<closeclaw_agent::registry::AgentRegistry>,
    pub skill_registry: &'a Arc<RwLock<Option<DiskSkillRegistry>>>,
    pub builtin_skill_registry: &'a Arc<BuiltinSkillRegistry>,
    pub tool_registry: &'a Arc<ToolRegistry>,
    pub session_manager: &'a Arc<SessionManager>,
    pub permission_engine: &'a Arc<tokio::sync::RwLock<PermissionEngine>>,
    pub approval_flow: &'a Arc<tokio::sync::Mutex<ApprovalFlow>>,
    pub confirm_flow: &'a Arc<PlanExecConfirmFlow>,
    pub gateway: &'a Arc<Gateway>,
    pub slash_registry: &'a Arc<closeclaw_slash::registry::HandlerRegistry>,
    pub shared_cache: &'a Arc<RwLock<SectionCache>>,
}

/// Components created in Phase 2 (registries), packed so the startup
/// orchestration steps of `start_with_engine_and_pid_file` can pass them
/// between each other without exceeding the 6-parameter limit.
pub(crate) struct Phase2Components {
    pub agent_registry: Arc<closeclaw_agent::registry::AgentRegistry>,
    pub skill_registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
    pub tool_registry: Arc<ToolRegistry>,
    pub shared_cache: Arc<RwLock<SectionCache>>,
    pub session_config_provider: Arc<dyn SessionConfigProvider>,
    pub llm_registry: Arc<LLMRegistry>,
    pub fallback_client: Arc<UnifiedFallbackClient>,
    pub permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    pub plan_archive_shutdown_tx: watch::Sender<()>,
    pub plan_archive_sweeper_handle: tokio::task::JoinHandle<()>,
}

/// Components created in Phase 3 (core services), packed for the startup
/// orchestration steps. The shutdown handle is Arc-shared here because the
/// orchestration wires clones into Gateway / SessionManager and keeps the
/// original on [`Daemon`].
pub(crate) struct Phase3Components {
    pub gateway: Arc<Gateway>,
    pub session_manager: Arc<SessionManager>,
    pub shutdown: Arc<crate::shutdown::ShutdownHandle>,
    pub dirty_sessions: Vec<String>,
    pub slash_registry: Arc<closeclaw_slash::registry::HandlerRegistry>,
    pub media_cleanup_handle: Option<closeclaw_tasks::media_cleanup::MediaCleanupHandle>,
}

/// Wiring outputs of Phase 4 (ApprovalFlow + BuiltinSkillRegistry).
pub(crate) struct Phase4Components {
    pub approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    pub builtin_skill_registry: Arc<BuiltinSkillRegistry>,
}

/// System-level background services spawned in Phase 5, packed for the
/// startup orchestration steps; unpacked into [`Daemon`] fields on
/// construction (field names mirror the [`Daemon`] fields they fill).
pub(crate) struct Phase5Background {
    pub sweeper_shutdown_tx: watch::Sender<()>,
    pub announce_shutdown_tx: watch::Sender<()>,
    pub dreaming_scheduler_shutdown_tx: watch::Sender<()>,
    pub config_watcher: config_watcher::ConfigWatcherHandle,
    pub archive_sweeper_handle: tokio::task::JoinHandle<()>,
    pub announce_sweeper_handle: tokio::task::JoinHandle<()>,
    pub dreaming_scheduler_handle: tokio::task::JoinHandle<()>,
    pub spawn_controller: Arc<SpawnController>,
    pub system_prompt_builder: Arc<dyn closeclaw_common::SystemPromptBuilder>,
    pub restart_rx: tokio::sync::mpsc::Receiver<String>,
}

/// RPC servers started in Phase 6 (Admin RPC + Chat RPC), packed for the
/// startup orchestration steps.
pub(crate) struct Phase6RpcServers {
    pub admin_handle: tokio::task::JoinHandle<()>,
    pub admin_socket_path: PathBuf,
    pub chat_handle: tokio::task::JoinHandle<()>,
    pub chat_socket_path: PathBuf,
    pub admin_restart_rx: tokio::sync::mpsc::Receiver<bool>,
}
