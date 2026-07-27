//! Bridge implementations — adapts daemon-crate concrete types to
//! `closeclaw_common` trait objects used by the gateway.
//!
//! Duplicated from root crate's `bridge.rs` because the daemon crate
//! cannot depend on the root crate (circular dependency).

use std::sync::Arc;

use async_trait::async_trait;

use crate::shutdown::ShutdownHandle as DaemonShutdownHandle;
use closeclaw_skills::BuiltinSkillRegistry;
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
// SkillListingProvider
// ═══════════════════════════════════════════════════════════════════════════

/// Newtype wrapper holding both `DiskSkillRegistry` and `BuiltinSkillRegistry`
/// to satisfy the orphan rule when implementing `SkillListingProvider`.
///
/// Merges listings from both registries with priority-based deduplication:
/// disk skills override builtin skills when names collide.
pub struct SkillListingProviderWrapper {
    pub disk: Arc<std::sync::RwLock<Option<closeclaw_skills::DiskSkillRegistry>>>,
    pub builtin: Arc<BuiltinSkillRegistry>,
}

impl SkillListingProviderWrapper {
    pub fn new(
        disk: Arc<std::sync::RwLock<Option<closeclaw_skills::DiskSkillRegistry>>>,
        builtin: Arc<BuiltinSkillRegistry>,
    ) -> Self {
        Self { disk, builtin }
    }

    /// Merge listings from both registries, deduplicating by name.
    ///
    /// Disk skills use `SkillSource` for priority ordering (Project > Agent >
    /// Global > ExtraDirs > Bundled). Builtin skills are treated as `Bundled`
    /// (lowest priority). When names collide, the higher-priority skill wins.
    fn merged_listing(&self, agent_id: Option<&str>, agent_skills: Option<&[String]>) -> String {
        // Collect disk skills
        let disk_lines: Vec<(String, u8)> = {
            self.disk
                .read()
                .ok()
                .and_then(|g| {
                    g.as_ref().map(|r| {
                        let all_skills = r.sorted_skills_for_listing(agent_skills);
                        let resolved_whitelist = agent_skills.map(|w| w.to_vec()).or_else(|| {
                            r.agent_skills_query()
                                .and_then(|q| q.get_agent_skills(agent_id.unwrap_or("")))
                        });
                        let use_whitelist = resolved_whitelist
                            .filter(|w| !(w.len() == 1 && w[0] == "*"))
                            .map(|w| w.iter().cloned().collect::<std::collections::HashSet<_>>());
                        all_skills
                            .into_iter()
                            .filter(|(skill, _)| {
                                skill.manifest.user_invocable
                                    && skill.manifest.paths.is_empty()
                                    && use_whitelist
                                        .as_ref()
                                        .map_or(true, |set| set.contains(&skill.manifest.name))
                            })
                            .map(|(skill, source)| {
                                let src = source as u8;
                                (
                                    closeclaw_skills::DiskSkillRegistry::render_single_listing(
                                        &skill,
                                    ),
                                    src,
                                )
                            })
                            .collect()
                    })
                })
                .unwrap_or_default()
        };

        // Collect builtin skills (Bundled = 4)
        let builtin_lines: Vec<(String, u8)> = {
            let rt = tokio::runtime::Handle::current();
            let entries = rt.block_on(self.builtin.sorted_skills());
            entries
                .into_iter()
                .filter(|(m, meta)| {
                    meta.user_invocable
                        && meta.paths.is_empty()
                        && agent_skills.map_or(true, |w| {
                            w == ["*"] || w.iter().any(|s| s.as_str() == m.name.as_str())
                        })
                })
                .map(|(m, meta)| {
                    let line = BuiltinSkillRegistry::render_single_listing(&m, &meta);
                    (line, 4u8) // Bundled priority
                })
                .collect()
        };

        // Merge: disk overrides builtin on name collision
        let mut builtin_by_name: std::collections::HashMap<String, (String, u8)> = builtin_lines
            .into_iter()
            .map(|(line, pri)| (extract_name(&line), (line, pri)))
            .collect();
        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<(String, u8)> = Vec::new();

        for (line, src) in disk_lines {
            let name = extract_name(&line);
            seen.insert(name);
            merged.push((line, src));
        }
        for (name, (line, pri)) in builtin_by_name.drain() {
            if !seen.contains(&name) {
                merged.push((line, pri));
            }
        }

        merged.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| extract_name(&a.0).cmp(&extract_name(&b.0)))
        });
        merged
            .into_iter()
            .map(|(line, _)| line)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Merge conditional matches from both registries, deduplicating by name.
    fn merged_conditional_matches(
        &self,
        paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        let mut disk_matches: Vec<closeclaw_common::ConditionalSkillMatch> = self
            .disk
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|r| r.find_conditional_matches(paths)))
            .unwrap_or_default();

        let disk_names: std::collections::HashSet<String> =
            disk_matches.iter().map(|m| m.name.clone()).collect();

        let builtin_matches = {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(self.builtin.find_conditional_matches(paths))
        };

        // Builtin matches that don't collide with disk matches
        for m in builtin_matches {
            if !disk_names.contains(&m.name) {
                disk_matches.push(m);
            }
        }

        disk_matches
    }
}

/// Extract the skill name from a listing line.
///
/// Listing lines have the format `- **{name}**: ...`.
fn extract_name(line: &str) -> String {
    line.trim_start_matches("- **")
        .split_once("**:")
        .map(|(name, _)| name.to_string())
        .unwrap_or_default()
}

impl closeclaw_common::SkillListingProvider for SkillListingProviderWrapper {
    fn generate_listing(&self, agent_id: Option<&str>, agent_skills: Option<&[String]>) -> String {
        self.merged_listing(agent_id, agent_skills)
    }

    fn generate_listing_excluding_conditional(
        &self,
        agent_id: Option<&str>,
        agent_skills: Option<&[String]>,
    ) -> String {
        self.merged_listing(agent_id, agent_skills)
    }

    fn find_conditional_matches(
        &self,
        paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        self.merged_conditional_matches(paths)
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
