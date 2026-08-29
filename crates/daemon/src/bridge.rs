//! Bridge implementations — adapts daemon-crate concrete types to
//! `closeclaw_common` trait objects used by the gateway.
//!
//! Duplicated from root crate's `bridge.rs` because the daemon crate
//! cannot depend on the root crate (circular dependency).

use std::sync::Arc;

use async_trait::async_trait;

use crate::shutdown::ShutdownHandle as DaemonShutdownHandle;
use closeclaw_skills::BuiltinSkillRegistry;

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
    ///
    /// When `exclude_conditional` is `true`, skills with non-empty `paths`
    /// are excluded. When `false`, all qualifying skills are included so
    /// the caller can compute the conditional-activation diff.
    fn merged_listing(
        &self,
        agent_id: Option<&str>,
        agent_skills: Option<&[String]>,
        exclude_conditional: bool,
    ) -> String {
        // Unified whitelist resolution: explicit agent_skills takes priority,
        // falling back to agent_skills_query from the disk registry.
        let resolved_whitelist = agent_skills.map(|w| w.to_vec()).or_else(|| {
            self.disk.read().ok().and_then(|g| {
                g.as_ref().and_then(|r| {
                    r.agent_skills_query()
                        .and_then(|q| q.get_agent_skills(agent_id.unwrap_or("")))
                })
            })
        });
        let resolved_ref = resolved_whitelist.as_deref();

        let disk = self.collect_disk_listings(agent_id, resolved_ref, exclude_conditional);
        let builtin = self.collect_builtin_listings(resolved_ref, exclude_conditional);
        Self::merge_and_sort_listings(disk, builtin)
    }

    /// Collect listing entries from the disk skill registry.
    ///
    /// Delegates to [`DiskSkillRegistry::listing_entries`] which handles
    /// `user_invocable` / whitelist filtering and `(source, name)` sorting.
    ///
    /// When `exclude_conditional` is `true`, conditional skills are
    /// excluded. When `false`, all qualifying skills are included.
    fn collect_disk_listings(
        &self,
        agent_id: Option<&str>,
        resolved_whitelist: Option<&[String]>,
        exclude_conditional: bool,
    ) -> Vec<(String, u8)> {
        let _ = agent_id; // retained for API symmetry; whitelist already resolved by caller
        self.disk
            .read()
            .ok()
            .and_then(|g| {
                g.as_ref().map(|r| {
                    r.listing_entries(resolved_whitelist, exclude_conditional)
                        .into_iter()
                        .map(|(line, source)| (line, source as u8))
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    /// Collect listing entries from the builtin skill registry.
    ///
    /// Delegates to [`BuiltinSkillRegistry::listing_entries`] for
    /// `user_invocable` / whitelist filtering and sorted output.
    fn collect_builtin_listings(
        &self,
        resolved_whitelist: Option<&[String]>,
        exclude_conditional: bool,
    ) -> Vec<(String, u8)> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(
            self.builtin
                .listing_entries(resolved_whitelist, exclude_conditional),
        )
    }

    /// Merge two sorted listing vectors, deduplicating by skill name.
    ///
    /// Disk entries take precedence over builtin entries when names
    /// collide. The final output is sorted by `(priority, name)`.
    fn merge_and_sort_listings(disk: Vec<(String, u8)>, builtin: Vec<(String, u8)>) -> String {
        let mut builtin_by_name: std::collections::HashMap<String, (String, u8)> = builtin
            .into_iter()
            .map(|(line, pri)| (extract_name(&line), (line, pri)))
            .collect();
        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<(String, u8)> = Vec::new();

        for (line, src) in disk {
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
        self.merged_listing(agent_id, agent_skills, false)
    }

    fn generate_listing_excluding_conditional(
        &self,
        agent_id: Option<&str>,
        agent_skills: Option<&[String]>,
    ) -> String {
        self.merged_listing(agent_id, agent_skills, true)
    }

    fn find_conditional_matches(
        &self,
        paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        self.merged_conditional_matches(paths)
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

// ═══════════════════════════════════════════════════════════════════════════
// Tests — SkillListingProviderWrapper integration
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::AgentSkillsQuery;
    use closeclaw_common::SkillListingProvider;
    use closeclaw_skills::disk::types::{DiskSkill, SkillSource};
    use closeclaw_skills::DiskSkillRegistry;
    use closeclaw_skills::{SkillListingMeta, SkillManifest};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    /// Run a closure on a dedicated thread with a tokio runtime context
    /// established via `enter()`. This allows `Handle::current().block_on()`
    /// inside `collect_builtin_listings` to work without panicking (unlike
    /// `block_on` within `block_on`).
    fn run_with_runtime<F>(f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let _guard = rt.enter();
                f();
            })
            .join()
            .expect("test thread panicked")
        })
    }

    // ------------------------------------------------------------------
    // Test helpers
    // ------------------------------------------------------------------

    /// Mock `AgentSkillsQuery` that returns a fixed whitelist for each agent.
    struct MockAgentSkillsQuery {
        /// agent_id → skills whitelist
        skills: HashMap<String, Vec<String>>,
    }

    impl MockAgentSkillsQuery {
        fn new(skills: HashMap<String, Vec<String>>) -> Self {
            Self { skills }
        }
    }

    impl AgentSkillsQuery for MockAgentSkillsQuery {
        fn get_agent_skills(&self, agent_id: &str) -> Option<Vec<String>> {
            self.skills.get(agent_id).cloned()
        }
    }

    /// Helper: build a `DiskSkill` with the given source and name.
    fn make_disk_skill(
        source: SkillSource,
        name: &str,
        user_invocable: bool,
        paths: Vec<String>,
    ) -> DiskSkill {
        DiskSkill {
            source,
            manifest: closeclaw_skills::disk::types::SkillManifest {
                name: name.to_string(),
                description: format!("disk skill {name}"),
                when_to_use: String::new(),
                context: Default::default(),
                effort: Default::default(),
                paths,
                user_invocable,
            },
            readme_path: PathBuf::from(format!("/tmp/{name}/SKILL.md")),
            skill_dir: PathBuf::from(format!("/tmp/{name}")),
        }
    }

    /// Helper: build a mock builtin skill with the given name and meta.
    fn make_builtin_skill(
        name: &str,
        user_invocable: bool,
        paths: Vec<String>,
    ) -> Arc<dyn closeclaw_skills::Skill> {
        struct MockBuiltin {
            name: String,
            meta: SkillListingMeta,
        }

        #[async_trait]
        impl closeclaw_skills::Skill for MockBuiltin {
            fn manifest(&self) -> SkillManifest {
                SkillManifest {
                    name: self.name.clone(),
                    version: "1.0.0".into(),
                    description: format!("builtin skill {}", self.name),
                    author: None,
                    dependencies: vec![],
                }
            }
            fn body(&self) -> &str {
                "mock body"
            }
            fn listing_meta(&self) -> SkillListingMeta {
                self.meta.clone()
            }
        }

        Arc::new(MockBuiltin {
            name: name.to_string(),
            meta: SkillListingMeta {
                when_to_use: String::new(),
                user_invocable,
                paths,
                effort: Default::default(),
            },
        })
    }

    /// Helper: create a `DiskSkillRegistry` with the given skills.
    fn make_disk_registry(skills: Vec<DiskSkill>) -> DiskSkillRegistry {
        closeclaw_skills::DiskSkillRegistry::new(skills)
    }

    /// Helper: create a `SkillListingProviderWrapper` from disk and builtin
    /// registries.
    fn make_wrapper(
        disk: DiskSkillRegistry,
        builtin: Arc<closeclaw_skills::BuiltinSkillRegistry>,
    ) -> SkillListingProviderWrapper {
        SkillListingProviderWrapper::new(Arc::new(std::sync::RwLock::new(Some(disk))), builtin)
    }

    // ------------------------------------------------------------------
    // Test 1: Agent whitelist filters builtin skills
    // ------------------------------------------------------------------

    #[test]
    fn test_whitelist_filters_builtin_skills() {
        run_with_runtime(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let builtin = rt.block_on(async {
                Arc::new(
                    closeclaw_skills::BuiltinSkillRegistry::from_skills(vec![
                        make_builtin_skill("alpha", true, vec![]),
                        make_builtin_skill("beta", true, vec![]),
                        make_builtin_skill("gamma", true, vec![]),
                    ])
                    .await,
                )
            });
            let disk = make_disk_registry(vec![]);
            let wrapper = make_wrapper(disk, builtin);

            // No whitelist → all builtin skills shown
            let listing = wrapper.generate_listing(None, None);
            assert!(listing.contains("alpha"));
            assert!(listing.contains("beta"));
            assert!(listing.contains("gamma"));

            // Whitelist with only beta → only beta shown
            let listing = wrapper.generate_listing(None, Some(&["beta".to_string()]));
            assert!(!listing.contains("alpha"));
            assert!(listing.contains("beta"));
            assert!(!listing.contains("gamma"));

            // Wildcard "*" → all shown
            let listing = wrapper.generate_listing(None, Some(&["*".to_string()]));
            assert!(listing.contains("alpha"));
            assert!(listing.contains("beta"));
            assert!(listing.contains("gamma"));
        });
    }

    // ------------------------------------------------------------------
    // Test 2: Agent skills query fallback for builtin filtering
    // ------------------------------------------------------------------

    #[test]
    fn test_builtin_filtered_via_agent_skills_query() {
        run_with_runtime(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let builtin = rt.block_on(async {
                Arc::new(
                    closeclaw_skills::BuiltinSkillRegistry::from_skills(vec![
                        make_builtin_skill("alpha", true, vec![]),
                        make_builtin_skill("beta", true, vec![]),
                        make_builtin_skill("gamma", true, vec![]),
                    ])
                    .await,
                )
            });

            // Simulate agent_skills_query returning whitelist ["alpha", "gamma"]
            // by injecting the query into the disk registry.
            let mut disk_reg = closeclaw_skills::DiskSkillRegistry::new(vec![]);
            disk_reg.set_agent_skills_query(Arc::new(MockAgentSkillsQuery::new(HashMap::from([
                (
                    "agent1".to_string(),
                    vec!["alpha".to_string(), "gamma".to_string()],
                ),
            ]))));
            let wrapper = make_wrapper(disk_reg, builtin);

            // agent_skills=None → unified resolver falls back to query
            let listing = wrapper.generate_listing(Some("agent1"), None);
            assert!(listing.contains("alpha"));
            assert!(!listing.contains("beta"));
            assert!(listing.contains("gamma"));
        });
    }

    // ------------------------------------------------------------------
    // Test 3: Disk skills override builtin skills with same name
    // ------------------------------------------------------------------

    #[test]
    fn test_disk_overrides_builtin() {
        run_with_runtime(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let builtin = rt.block_on(async {
                Arc::new(
                    closeclaw_skills::BuiltinSkillRegistry::from_skills(vec![
                        make_builtin_skill("shared_skill", true, vec![]),
                        make_builtin_skill("other_skill", true, vec![]),
                    ])
                    .await,
                )
            });
            let disk = make_disk_registry(vec![make_disk_skill(
                SkillSource::Project,
                "shared_skill",
                true,
                vec![],
            )]);

            let wrapper = make_wrapper(disk, builtin);
            let listing = wrapper.generate_listing(None, None);

            // shared_skill appears once (disk version wins)
            let count = listing.matches("- **shared_skill**: ").count();
            assert_eq!(count, 1, "shared_skill should appear exactly once");
            // other_skill (builtin only) still appears
            assert!(listing.contains("other_skill"));
        });
    }

    // ------------------------------------------------------------------
    // Test 4: Cross-registry sorting by source priority then name
    // ------------------------------------------------------------------

    #[test]
    fn test_cross_registry_sorting() {
        run_with_runtime(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let builtin =
                rt.block_on(async {
                    Arc::new(
                        closeclaw_skills::BuiltinSkillRegistry::from_skills(vec![
                            make_builtin_skill("b_skill", true, vec![]),
                        ])
                        .await,
                    )
                });
            // Disk: Agent-priority "z_skill", Project-priority "a_skill"
            let disk = make_disk_registry(vec![
                make_disk_skill(SkillSource::Agent, "z_skill", true, vec![]),
                make_disk_skill(SkillSource::Project, "a_skill", true, vec![]),
            ]);

            let wrapper = make_wrapper(disk, builtin);
            let listing = wrapper.generate_listing(None, None);

            // Expected order: a_skill (Project=0), z_skill (Agent=1), b_skill (Bundled=4)
            let a_pos = listing.find("a_skill").unwrap();
            let z_pos = listing.find("z_skill").unwrap();
            let b_pos = listing.find("b_skill").unwrap();
            assert!(
                a_pos < z_pos && z_pos < b_pos,
                "expected a_skill < z_skill < b_skill but got a={a_pos} z={z_pos} b={b_pos}"
            );
        });
    }

    // ------------------------------------------------------------------
    // Test 5: Conditional skill exclusion
    // ------------------------------------------------------------------

    #[test]
    fn test_conditional_exclusion() {
        run_with_runtime(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let builtin = rt.block_on(async {
                Arc::new(
                    closeclaw_skills::BuiltinSkillRegistry::from_skills(vec![
                        make_builtin_skill("regular", true, vec![]),
                        make_builtin_skill("conditional", true, vec!["**/*.rs".to_string()]),
                    ])
                    .await,
                )
            });
            let disk = make_disk_registry(vec![]);
            let wrapper = make_wrapper(disk, builtin);

            // generate_listing_excluding_conditional → conditional excluded
            let listing = wrapper.generate_listing_excluding_conditional(None, None);
            assert!(listing.contains("regular"));
            assert!(!listing.contains("conditional"));
        });
    }

    // ------------------------------------------------------------------
    // Test 6: SP rebuild path — listing includes activated conditionals,
    //         excluding does not
    // ------------------------------------------------------------------

    #[test]
    fn test_sp_rebuild_path() {
        run_with_runtime(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let builtin = rt.block_on(async {
                Arc::new(
                    closeclaw_skills::BuiltinSkillRegistry::from_skills(vec![
                        make_builtin_skill("always", true, vec![]),
                        make_builtin_skill("conditional", true, vec!["**/*.rs".to_string()]),
                    ])
                    .await,
                )
            });
            let disk = make_disk_registry(vec![]);
            let wrapper = make_wrapper(disk, builtin);

            // Full listing (generate_listing) includes conditional
            let full = wrapper.generate_listing(None, None);
            assert!(full.contains("always"));
            assert!(full.contains("conditional"));

            // Excluding listing does not include conditional
            let base = wrapper.generate_listing_excluding_conditional(None, None);
            assert!(base.contains("always"));
            assert!(!base.contains("conditional"));
        });
    }

    // ------------------------------------------------------------------
    // Test 7: Whitelist filters both disk and builtin skills
    // ------------------------------------------------------------------

    #[test]
    fn test_whitelist_filters_disk_and_builtin() {
        run_with_runtime(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let builtin = rt.block_on(async {
                Arc::new(
                    closeclaw_skills::BuiltinSkillRegistry::from_skills(vec![
                        make_builtin_skill("builtin_only", true, vec![]),
                        make_builtin_skill("shared", true, vec![]),
                    ])
                    .await,
                )
            });
            let disk = make_disk_registry(vec![
                make_disk_skill(SkillSource::Global, "disk_only", true, vec![]),
                make_disk_skill(SkillSource::Global, "shared", true, vec![]),
            ]);
            let wrapper = make_wrapper(disk, builtin);

            // Whitelist ["shared"] → only shared appears (from disk)
            let listing = wrapper.generate_listing(None, Some(&["shared".to_string()]));
            assert!(listing.contains("shared"));
            assert!(!listing.contains("disk_only"));
            assert!(!listing.contains("builtin_only"));
        });
    }

    // ------------------------------------------------------------------
    // Test 8: find_conditional_matches from merged registries
    // ------------------------------------------------------------------

    #[test]
    fn test_find_conditional_matches_merged() {
        run_with_runtime(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let builtin = rt.block_on(async {
                Arc::new(
                    closeclaw_skills::BuiltinSkillRegistry::from_skills(vec![
                        make_builtin_skill("rust_skill", true, vec!["**/*.rs".to_string()]),
                        make_builtin_skill("txt_skill", true, vec!["**/*.txt".to_string()]),
                    ])
                    .await,
                )
            });
            let disk = make_disk_registry(vec![]);
            let wrapper = make_wrapper(disk, builtin);

            let matches = wrapper.find_conditional_matches(&[PathBuf::from("src/main.rs")]);
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].name, "rust_skill");
        });
    }

    // ------------------------------------------------------------------
    // Test 9: empty registries produce empty listing
    // ------------------------------------------------------------------

    #[test]
    fn test_empty_registries() {
        run_with_runtime(|| {
            let disk = make_disk_registry(vec![]);
            let builtin = Arc::new(closeclaw_skills::BuiltinSkillRegistry::new());

            let wrapper = make_wrapper(disk, builtin);
            assert!(wrapper.generate_listing(None, None).is_empty());
            assert!(wrapper
                .generate_listing_excluding_conditional(None, None)
                .is_empty());
        });
    }
}
