//! Bridge implementations — adapts main-crate concrete types to
//! `closeclaw_common` trait objects used by the gateway.

use std::sync::Arc;

use async_trait::async_trait;

use closeclaw_daemon::shutdown::ShutdownHandle as DaemonShutdownHandle;

// ═══════════════════════════════════════════════════════════════════════════
// SkillRegistryQuery — newtype wrapper (orphan rule)
// ═══════════════════════════════════════════════════════════════════════════

/// Newtype wrapper around `Arc<RwLock<Option<DiskSkillRegistry>>>` to
/// satisfy the orphan rule when implementing `SkillRegistryQuery`.
pub struct SkillRegistryWrapper(
    pub Arc<std::sync::RwLock<Option<closeclaw_skills::DiskSkillRegistry>>>,
);

#[async_trait]
impl closeclaw_common::skill_registry::SkillRegistryQuery for SkillRegistryWrapper {
    async fn has_skill(&self, name: &str) -> bool {
        self.0
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|r| r.contains(name)))
            .unwrap_or(false)
    }

    async fn list_skills(&self) -> Vec<String> {
        self.0
            .read()
            .ok()
            .and_then(|g| {
                g.as_ref()
                    .map(|r| r.list().into_iter().map(String::from).collect())
            })
            .unwrap_or_default()
    }

    async fn list_skills_for_agent(&self, agent_skills: Option<&[String]>) -> Vec<String> {
        self.0
            .read()
            .ok()
            .and_then(|g| {
                g.as_ref().map(|r| {
                    let all = r.list();
                    match agent_skills {
                        Some(skills) if skills.len() == 1 && skills[0] == "*" => {
                            all.into_iter().map(String::from).collect()
                        }
                        Some([]) => all.into_iter().map(String::from).collect(),
                        Some(skills) => {
                            let set: std::collections::HashSet<&str> =
                                skills.iter().map(|s| s.as_str()).collect();
                            all.into_iter()
                                .filter(|name| set.contains(*name))
                                .map(String::from)
                                .collect()
                        }
                        None => all.into_iter().map(String::from).collect(),
                    }
                })
            })
            .unwrap_or_default()
    }

    fn generate_listing(&self, agent_id: Option<&str>, agent_skills: Option<&[String]>) -> String {
        self.0
            .read()
            .ok()
            .and_then(|g| {
                g.as_ref()
                    .map(|r| r.generate_listing(agent_id, agent_skills))
            })
            .unwrap_or_default()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Slash router — CLI chat slash assembly (composition root)
// ═══════════════════════════════════════════════════════════════════════════

/// Assemble the concrete slash router injected into the CLI chat Gateway.
///
/// The CLI crate only consumes `closeclaw_common::SlashRouter`; the concrete
/// handler set (and its registration order) is owned here, in the
/// composition root. `sm_query` is the chat process's `SessionManager`,
/// supplied by `build_gateway` once the session manager exists.
pub fn build_chat_slash_router(
    sm_query: Arc<dyn closeclaw_common::SlashSessionQuery>,
) -> Arc<dyn closeclaw_common::SlashRouter> {
    let registry = Arc::new(closeclaw_slash::registry::HandlerRegistry::new());
    registry.register(Arc::new(closeclaw_slash::handlers::CompactHandler));
    registry.register(Arc::new(closeclaw_slash::handlers_session::StopHandler));
    registry.register(Arc::new(
        closeclaw_slash::handlers_session::VerboseHandler::new(sm_query),
    ));
    Arc::new(closeclaw_slash::dispatcher::SlashDispatcher::from_shared(
        registry,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Processor chain — CLI chat gateway assembly (composition root)
// ═══════════════════════════════════════════════════════════════════════════

/// Assemble the concrete processor chain injected into the CLI chat Gateway.
///
/// The cli crate only consumes `closeclaw_common::processor::ProcessorChain`;
/// the concrete inbound/outbound processor set is owned here, in the
/// composition root (delegated to the daemon-side single implementation).
pub fn build_chat_processor_chain(
    config: &closeclaw_gateway::GatewayConfig,
) -> Arc<dyn closeclaw_common::processor::ProcessorChain> {
    closeclaw_daemon::processor_registry::build_processor_chain(config)
}

// ═══════════════════════════════════════════════════════════════════════════
// ShutdownHandle conversion
// ═══════════════════════════════════════════════════════════════════════════

// DaemonShutdownMode is now a re-export of closeclaw_common::ShutdownMode,
// so no conversion is needed.

/// Create a `closeclaw_gateway::shutdown_handle::ShutdownHandle` from the daemon's
/// `ShutdownHandle`. The common handle wraps the daemon's handle as a
/// `dyn ShutdownSignal`.
pub fn common_shutdown_handle(
    daemon_handle: &DaemonShutdownHandle,
) -> Arc<closeclaw_gateway::shutdown_handle::ShutdownHandle> {
    Arc::new(closeclaw_gateway::shutdown_handle::ShutdownHandle::new(
        Arc::new(daemon_handle.clone()),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use closeclaw_common::{PendingMessage, PlanState, SessionMode, SlashSessionQuery};

    use super::build_chat_slash_router;

    /// Fake [`SlashSessionQuery`] handed to `VerboseHandler` at assembly
    /// time. These tests only inspect the assembled router (handler set,
    /// immediacy) and never execute a handler, so any actual session query
    /// is an unexpected call and must fail loudly rather than answer with a
    /// plausible default.
    struct FakeSessionQuery;

    #[async_trait]
    impl SlashSessionQuery for FakeSessionQuery {
        async fn get_plan_state(&self, _: &str) -> Option<PlanState> {
            unimplemented!()
        }
        async fn set_plan_state(&self, _: &str, _: PlanState) {
            unimplemented!()
        }
        async fn push_pending_message(&self, _: &str, _: PendingMessage) -> Result<(), String> {
            unimplemented!()
        }
        async fn trigger_manual_background(&self, _: &str) -> Result<bool, String> {
            unimplemented!()
        }
        async fn set_workflow_run(
            &self,
            _: &str,
            _: Option<Box<dyn std::any::Any + Send + Sync>>,
        ) -> Result<(), String> {
            unimplemented!()
        }
        async fn get_active_workflow_run_phase(&self, _: &str) -> Option<String> {
            unimplemented!()
        }
        async fn invalidate_static_cache(&self) {
            unimplemented!()
        }
        async fn rebuild_system_prompt_for_session(&self, _: &str) {
            unimplemented!()
        }
        async fn add_system_append(&self, _: &str, _: String) {
            unimplemented!()
        }
        async fn add_system_injection_append(&self, _: &str, _: String) {
            unimplemented!()
        }
        async fn get_model(&self, _: &str) -> Option<String> {
            unimplemented!()
        }
        async fn get_reasoning_level(&self, _: &str) -> Option<String> {
            unimplemented!()
        }
        async fn get_verbosity_level(&self, _: &str) -> Option<String> {
            unimplemented!()
        }
        async fn get_session_mode(&self, _: &str) -> Option<SessionMode> {
            unimplemented!()
        }
        async fn get_workdir(&self, _: &str) -> Option<std::path::PathBuf> {
            unimplemented!()
        }
        async fn get_system_appends(&self, _: &str) -> Vec<String> {
            unimplemented!()
        }
        async fn set_workdir(&self, _: &str, _: std::path::PathBuf) {
            unimplemented!()
        }
        async fn is_llm_busy(&self, _: &str) -> bool {
            unimplemented!()
        }
        async fn get_stats(&self, _: &str) -> Option<(usize, usize, usize, usize)> {
            unimplemented!()
        }
        async fn get_last_cache_break(&self, _: &str) -> Option<String> {
            unimplemented!()
        }
        async fn get_active_child_count(&self, _: &str) -> usize {
            unimplemented!()
        }
    }

    /// `build_chat_slash_router` owns the handler set after the migration
    /// out of `cli::chat`; it must keep exactly the pre-migration set
    /// `{compact, stop, verbose}`, each command name resolving to the
    /// handler that claims that name (a mis-registered handler would fail
    /// the `commands()` projection), and no other command routable — i.e.
    /// the set invariant survives the composition-root move.
    #[tokio::test]
    async fn chat_slash_router_registers_exactly_compact_stop_verbose() {
        let router = build_chat_slash_router(Arc::new(FakeSessionQuery));

        for cmd in ["compact", "stop", "verbose"] {
            let handler = router
                .get_handler(cmd)
                .unwrap_or_else(|| panic!("handler for `/{cmd}` must be registered"));
            let expected: &[&str] = &[cmd];
            assert_eq!(
                handler.commands(),
                expected,
                "`/{cmd}` must resolve to the handler claiming that command name"
            );
        }

        // Commands served by other assemblies (daemon chat, admin) must stay
        // unreachable from the CLI chat router.
        for cmd in ["status", "mode", "new"] {
            assert!(
                router.get_handler(cmd).is_none(),
                "`/{cmd}` is outside the CLI chat set and must stay unregistered"
            );
        }
    }

    /// Immediacy is the user-visible projection of the assembled handler
    /// set: `/stop` and `/verbose` answer while the LLM is busy, `/compact`
    /// does not — matching the pre-migration `cli::chat` assembly, where the
    /// handler registered under a command name decides this flag.
    #[tokio::test]
    async fn chat_slash_router_is_immediate_matches_original_assembly() {
        let router = build_chat_slash_router(Arc::new(FakeSessionQuery));

        assert!(
            router.is_immediate("/stop"),
            "/stop must be answered while the LLM is busy"
        );
        assert!(
            router.is_immediate("/verbose full"),
            "/verbose must be answered while the LLM is busy (args ignored)"
        );
        assert!(
            !router.is_immediate("/compact"),
            "/compact must wait for an idle LLM turn"
        );
        assert!(
            !router.is_immediate("/compact focus"),
            "/compact immediacy must not depend on its arguments"
        );
        // Unregistered commands and plain text are never immediate — they
        // never reach a handler, so nothing can claim immediacy for them.
        assert!(!router.is_immediate("/status"));
        assert!(!router.is_immediate("plain text"));
    }
}
