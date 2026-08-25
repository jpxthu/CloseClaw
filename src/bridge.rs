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
