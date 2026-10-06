//! Plugin initialization: terminal plugin and slash dispatcher registration.
//!
//! Extracted from `mod.rs` to keep source files within the CONTRIBUTING.md
//! limits (`mod.rs` only holds `pub use` / `pub mod` re-exports).

use crate::Daemon;
use std::sync::Arc;
use tracing::{info, warn};

// --- Service init helpers ---
impl Daemon {
    /// Initialize the terminal (CLI) IM plugin and register with Gateway.
    pub(crate) async fn init_terminal_plugin(gateway: &Arc<closeclaw_gateway::Gateway>) {
        use closeclaw_cli::terminal::TerminalPlugin;
        let plugin: Arc<dyn closeclaw_common::IMPlugin> = Arc::new(TerminalPlugin::new());
        gateway.register_plugin(plugin).await;
        info!("Terminal plugin registered");
    }

    /// Initialize the slash command dispatcher and register all handlers.
    ///
    /// Returns the shared [`HandlerRegistry`] so callers can later register
    /// additional handlers (e.g. [`SkillSlashHandler`]) after dependent
    /// registries are initialized.
    pub(crate) async fn init_slash_dispatcher(
        gateway: &Arc<closeclaw_gateway::Gateway>,
        session_manager: &Arc<closeclaw_gateway::SessionManager>,
    ) -> Arc<closeclaw_slash::registry::HandlerRegistry> {
        use closeclaw_slash::dispatcher::SlashDispatcher;
        use closeclaw_slash::handlers::{ReasoningHandler, SystemHandler, WorkdirHandler};
        use closeclaw_slash::handlers_bg::BackgroundHandler;
        use closeclaw_slash::handlers_permission::PermissionSlashHandler;
        use closeclaw_slash::handlers_user::UserSlashHandler;
        use closeclaw_slash::registry::HandlerRegistry;
        use closeclaw_slash::{
            ClearHandler, CompactHandler, ExecHandler, ExecuteHandler, HelpHandler, ModeHandler,
            NewSessionHandler, PlanBrowseHandler, PlanModeHandler, StatusHandler, StopHandler,
            VerboseHandler, WorkflowSlashHandler,
        };

        let sm_query: Arc<dyn closeclaw_common::SlashSessionQuery> = session_manager.clone();
        let slash_registry = Arc::new(HandlerRegistry::new());
        let registry_for_return = Arc::clone(&slash_registry);
        slash_registry.register(Arc::new(CompactHandler));
        slash_registry.register(Arc::new(ClearHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(ExecHandler));
        slash_registry.register(Arc::new(WorkdirHandler::new(Arc::clone(&sm_query))));
        let help_handler = HelpHandler::new(Arc::clone(&slash_registry));
        slash_registry.register(Arc::new(help_handler));
        slash_registry.register(Arc::new(ReasoningHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(VerboseHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(SystemHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(NewSessionHandler));
        slash_registry.register(Arc::new(StopHandler));
        slash_registry.register(Arc::new(StatusHandler::new(Arc::clone(&sm_query))));
        let plan_handler = Arc::new(PlanModeHandler::new(
            Arc::clone(&sm_query),
            to_session_identifier_format(closeclaw_config::IdentifierFormat::default()),
        ));
        slash_registry.register(plan_handler.clone() as Arc<dyn closeclaw_common::SlashHandler>);
        slash_registry.register(Arc::new(ModeHandler::with_handlers(
            Arc::clone(&sm_query),
            plan_handler,
        )));
        slash_registry.register(Arc::new(ExecuteHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(BackgroundHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(PlanBrowseHandler::new(Arc::clone(&sm_query))));
        slash_registry.register(Arc::new(PermissionSlashHandler));
        if let Some(config_dir) = gateway.get_config_dir().await {
            slash_registry.register(Arc::new(UserSlashHandler::new(config_dir)));
        }
        // Register `/workflow` slash handler (design doc §触发机制/§工具注册).
        // agent_workspace=None: resolved dynamically via get_workdir(session_id)
        // at handle()-time, consistent with session-layer workflow lookup.
        let global_workflows = match dirs::home_dir() {
            Some(home) => Some(home.join(".openclaw")),
            None => {
                warn!(
                    "global workflow directory unavailable: $HOME is not set — \
                     workflow lookup will only search agent workspace and builtins"
                );
                None
            }
        };
        slash_registry.register(Arc::new(WorkflowSlashHandler::new(
            Arc::clone(&sm_query),
            None,
            global_workflows,
        )));
        let slash_dispatcher = Arc::new(SlashDispatcher::from_shared(slash_registry))
            as Arc<dyn closeclaw_common::SlashRouter>;
        gateway.set_slash_dispatcher(slash_dispatcher).await;
        // PermissionEngine injection moved to init_phase_3_core_services
        // immediately after Gateway construction (dependency topology aligned).
        info!("Slash dispatcher installed");
        registry_for_return
    }
}

/// Map the config-side plan identifier format to the session-side enum.
///
/// `closeclaw-session` no longer depends on `closeclaw-config`, so the
/// config→session mapping lives at the call site (this module).
fn to_session_identifier_format(
    format: closeclaw_config::IdentifierFormat,
) -> closeclaw_session::plan_file::PlanIdentifierFormat {
    match format {
        closeclaw_config::IdentifierFormat::Timestamp => {
            closeclaw_session::plan_file::PlanIdentifierFormat::Timestamp
        }
        closeclaw_config::IdentifierFormat::RandomWords => {
            closeclaw_session::plan_file::PlanIdentifierFormat::RandomWords
        }
    }
}
