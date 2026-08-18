//! Daemon struct definitions, extracted from mod.rs to stay under the
//! 1000-line limit imposed by CONTRIBUTING.md.

use closeclaw_config::ConfigManager;
use closeclaw_gateway::SpawnController;
use closeclaw_gateway::{Gateway, SessionManager};
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::PermissionEngine;
use closeclaw_session::storage::SqliteStorage;
use closeclaw_skills::{BuiltinSkillRegistry, DiskSkillRegistry, SkillWatcherHandle};
use closeclaw_system_prompt::sections::SectionCache;
use closeclaw_tools::ToolRegistry;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::watch;

use crate::config_watcher;

/// Global daemon state
pub struct Daemon {
    pub gateway: Arc<Gateway>,
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
    /// Shutdown sender for PlanArchiveTask
    pub plan_archive_shutdown_tx: watch::Sender<()>,
    /// Shared skill registry, updated on hot reload
    pub skill_registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
    /// Builtin skill registry — compiled-in skills, not subject to hot reload
    pub builtin_skill_registry: Arc<BuiltinSkillRegistry>,
    /// Slash command handler registry — shared with SlashDispatcher;
    /// allows late registration of SkillSlashHandler after registries are ready.
    pub slash_registry: Arc<closeclaw_slash::registry::HandlerRegistry>,
    /// Skill file watcher handle (RAII: stops on drop)
    pub(crate) _skill_watcher: Option<SkillWatcherHandle>,
    /// Config file watcher handle (RAII: stops on drop)
    pub(crate) _config_watcher: Option<config_watcher::ConfigWatcherHandle>,
    /// Daemon-level approval orchestrator
    pub approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    /// Admin RPC server task handle (drop cancels the task)
    #[allow(dead_code)]
    pub(crate) admin_handle: Option<tokio::task::JoinHandle<()>>,
    /// Path to the admin RPC socket file (cleaned up on shutdown)
    pub(crate) admin_socket_path: PathBuf,
    /// Chat RPC server task handle (drop cancels the task)
    #[allow(dead_code)]
    pub(crate) chat_handle: Option<tokio::task::JoinHandle<()>>,
    /// Path to the chat RPC socket file (cleaned up on shutdown)
    pub(crate) chat_socket_path: PathBuf,
    /// Join handle for ArchiveSweeper background task
    pub(crate) archive_sweeper_handle: Option<tokio::task::JoinHandle<()>>,
    /// Join handle for AnnounceSweeper background task
    pub(crate) announce_sweeper_handle: Option<tokio::task::JoinHandle<()>>,
    /// Join handle for DreamingScheduler background task
    pub(crate) dreaming_scheduler_handle: Option<tokio::task::JoinHandle<()>>,
    /// Join handle for PlanArchiveTask background task
    pub(crate) plan_archive_task_handle: Option<tokio::task::JoinHandle<()>>,
    /// SpawnController reference — manages sub-agent lifecycle
    pub spawn_controller: Option<Arc<SpawnController>>,
    /// SystemPromptBuilder reference — static-layer prompt construction
    pub system_prompt_builder: Option<Arc<dyn closeclaw_common::SystemPromptBuilder>>,
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
    pub gateway: &'a Arc<Gateway>,
    pub slash_registry: &'a Arc<closeclaw_slash::registry::HandlerRegistry>,
    pub shared_cache: &'a Arc<RwLock<SectionCache>>,
}
