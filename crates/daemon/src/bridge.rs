//! Bridge implementations — adapts daemon-crate concrete types to
//! `closeclaw_common` trait objects used by the gateway.
//!
//! Duplicated from root crate's `bridge.rs` because the daemon crate
//! cannot depend on the root crate (circular dependency).

use std::sync::Arc;

use async_trait::async_trait;

use crate::shutdown::ShutdownHandle as DaemonShutdownHandle;
use closeclaw_slash::SlashDispatcher;

// ═══════════════════════════════════════════════════════════════════════════
// ShutdownHandle conversion
// ═══════════════════════════════════════════════════════════════════════════

/// Create a `closeclaw_gateway::shutdown_handle::ShutdownHandle` from the daemon's
/// `ShutdownHandle`.
pub fn common_shutdown_handle(
    daemon_handle: &DaemonShutdownHandle,
) -> Arc<closeclaw_gateway::shutdown_handle::ShutdownHandle> {
    Arc::new(closeclaw_gateway::shutdown_handle::ShutdownHandle::new(
        Arc::new(daemon_handle.clone()),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// SkillRegistryQuery
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
// SlashRouter adapter
// ═══════════════════════════════════════════════════════════════════════════

/// Newtype wrapper around `SlashDispatcher` to satisfy the orphan rule
/// when implementing `closeclaw_common::SlashRouter`.
pub struct SlashDispatcherWrapper(pub SlashDispatcher);

/// Thin wrapper converting `Arc<dyn SlashHandler>` to `Box<dyn SlashHandler>`
/// for the common `SlashRouter` trait.
struct SlashHandlerBox {
    inner: Arc<dyn closeclaw_common::slash_router::SlashHandler>,
}

#[async_trait]
impl closeclaw_common::slash_router::SlashHandler for SlashHandlerBox {
    fn commands(&self) -> &[&str] {
        self.inner.commands()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn immediate(&self, cmd: &str) -> bool {
        self.inner.immediate(cmd)
    }
    fn requires_permission(&self) -> bool {
        self.inner.requires_permission()
    }
    async fn handle(
        &self,
        args: &str,
        ctx: &closeclaw_common::slash_router::SlashContext,
    ) -> closeclaw_common::slash_router::SlashResult {
        self.inner.handle(args, ctx).await
    }
}

#[async_trait]
impl closeclaw_common::slash_router::SlashRouter for SlashDispatcherWrapper {
    async fn dispatch(
        &self,
        content: &str,
        ctx: &closeclaw_common::slash_router::SlashContext,
    ) -> Option<closeclaw_common::slash_router::SlashResult> {
        Some(self.0.dispatch(content, ctx).await)
    }

    fn is_immediate(&self, command: &str) -> bool {
        self.0.is_immediate(command)
    }

    fn get_handler(
        &self,
        command: &str,
    ) -> Option<Box<dyn closeclaw_common::slash_router::SlashHandler>> {
        self.0.get_handler(command).map(|h| {
            Box::new(SlashHandlerBox { inner: h })
                as Box<dyn closeclaw_common::slash_router::SlashHandler>
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DaemonRunner — in-process daemon execution for --foreground mode
// ═══════════════════════════════════════════════════════════════════════════

/// Unit struct implementing [`closeclaw_cli::admin::DaemonRunner`].
///
/// Pass an instance to `handle_run` / `handle_run_foreground` so the CLI
/// can run the daemon in-process without spawning a subprocess (and without
/// a circular crate dependency on `closeclaw-daemon`).
pub struct DaemonRunnerImpl;

#[async_trait]
impl closeclaw_cli::admin::DaemonRunner for DaemonRunnerImpl {
    async fn start_and_run(&self, config_dir: &str) -> anyhow::Result<()> {
        use crate::Daemon;
        let mut daemon = Daemon::start(config_dir).await?;
        daemon.run().await
    }
}
