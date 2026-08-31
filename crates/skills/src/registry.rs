//! Skill Registry - manages skill registration and discovery

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::disk::types::SkillEffort;

/// Skill metadata
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

/// Metadata required for listing generation.
///
/// Builtin skills provide this so they can appear in the same
/// skill listing that disk-based skills already produce.
#[derive(Debug, Clone, Default)]
pub struct SkillListingMeta {
    /// When to use this skill (decision hint).
    pub when_to_use: String,
    /// Whether the skill can be invoked directly by a user.
    pub user_invocable: bool,
    /// File glob patterns for conditional activation.
    pub paths: Vec<String>,
    /// Estimated effort level.
    pub effort: SkillEffort,
}

/// Skill trait - implemented by each skill
#[async_trait]
pub trait Skill: Send + Sync {
    /// Get skill manifest
    fn manifest(&self) -> SkillManifest;

    /// Get skill prompt body text
    fn body(&self) -> &str;

    /// Execute the skill with the given arguments.
    ///
    /// Bundled skills override this to run native code logic and
    /// return structured results as a meta message. The default
    /// implementation delegates to [`body()`] for backward
    /// compatibility with existing skills.
    async fn execute(&self, args: Option<serde_json::Value>) -> Result<String, SkillError> {
        let _ = args;
        Ok(self.body().to_string())
    }

    /// Get listing metadata for this skill.
    ///
    /// Used by the listing generator to render builtin skills
    /// into the same format as disk-based skills.
    ///
    /// The default implementation returns a sentinel value with
    /// `user_invocable: false` so that non-user-visible skills do
    /// not need to override this method.
    fn listing_meta(&self) -> SkillListingMeta {
        SkillListingMeta::default()
    }
}

/// Builtin skill registry - manages all registered builtin skills
pub struct BuiltinSkillRegistry {
    skills: tokio::sync::RwLock<HashMap<String, Arc<dyn Skill>>>,
}

impl Default for BuiltinSkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BuiltinSkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Register a skill
    pub async fn register(&self, skill: Arc<dyn Skill>) {
        let mut skills = self.skills.write().await;
        skills.insert(skill.manifest().name.clone(), skill);
    }

    /// Get a skill by name
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Skill>> {
        let skills = self.skills.read().await;
        skills.get(name).cloned()
    }

    /// List all skill names
    pub async fn list(&self) -> Vec<String> {
        let skills = self.skills.read().await;
        skills.keys().cloned().collect()
    }

    /// Check if a skill exists
    pub async fn contains(&self, name: &str) -> bool {
        let skills = self.skills.read().await;
        skills.contains_key(name)
    }

    /// Unregister a skill
    pub async fn unregister(&self, name: &str) -> bool {
        let mut skills = self.skills.write().await;
        skills.remove(name).is_some()
    }

    /// Create a registry pre-populated with the given skills.
    pub async fn from_skills(skills: Vec<Arc<dyn Skill>>) -> Self {
        let registry = Self::new();
        for skill in skills {
            registry.register(skill).await;
        }
        registry
    }

    // -----------------------------------------------------------------------
    // Listing generation
    // -----------------------------------------------------------------------

    /// Return filtered, sorted, rendered listing entries suitable for
    /// merge into the combined listing.
    ///
    /// Each entry is `(listing_line, u8)`. The list is
    /// sorted by `(source, name)` for consistent merge ordering.
    /// All builtin skills share source value `4u8` (`SkillSource::Bundled`).
    ///
    /// When `exclude_conditional` is `true`, skills with non-empty
    /// `paths` are excluded. When `false`, all qualifying skills
    /// (including conditional) are included.
    pub async fn listing_entries(
        &self,
        skills_whitelist: Option<&[String]>,
        exclude_conditional: bool,
    ) -> Vec<(String, u8)> {
        let entries = self.sorted_skills().await;
        let use_whitelist = skills_whitelist
            .filter(|w| !(w.len() == 1 && w[0] == "*"))
            .map(|w| {
                w.iter()
                    .map(|s| s.as_str())
                    .collect::<std::collections::HashSet<_>>()
            });

        let mut filtered: Vec<(String, u8)> = entries
            .into_iter()
            .filter(|(m, meta)| {
                meta.user_invocable
                    && (!exclude_conditional || meta.paths.is_empty())
                    && match &use_whitelist {
                        Some(set) => set.contains(m.name.as_str()),
                        None => true,
                    }
            })
            .map(|(m, meta)| {
                let line = Self::render_single_listing(&m, &meta);
                (line, 4u8) // Bundled priority
            })
            .collect();

        filtered.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        filtered
    }

    /// Return the names of all registered skills with `user_invocable: true`.
    ///
    /// Used by [`SkillSlashHandler`] to register slash commands for
    /// builtin skills alongside disk-based skills.
    pub async fn user_invocable_names(&self) -> Vec<String> {
        let entries = self.sorted_skills().await;
        entries
            .iter()
            .filter(|(_, meta)| meta.user_invocable)
            .map(|(m, _)| m.name.clone())
            .collect()
    }

    /// Render a single builtin skill's listing line.
    ///
    /// Format matches [`DiskSkillRegistry::render_single_listing`]:
    /// `- **{name}**: {description} — {when_to_use} ⚡ auto-activates on: {paths} [effort: ...]`
    pub fn render_single_listing(manifest: &SkillManifest, meta: &SkillListingMeta) -> String {
        let when = if meta.when_to_use.is_empty() {
            String::new()
        } else {
            format!(" — {}", meta.when_to_use)
        };
        let paths_anno = if meta.paths.is_empty() {
            String::new()
        } else {
            format!(" ⚡ auto-activates on: {}", meta.paths.join(", "))
        };
        let effort_anno = match meta.effort {
            SkillEffort::Unknown => String::new(),
            effort => format!(" [effort: {}]", effort),
        };
        format!(
            "- **{}**: {}{}{}{}",
            manifest.name, manifest.description, when, paths_anno, effort_anno,
        )
    }

    /// Collects all skills with their metadata, sorted by name
    /// (all builtin skills share the same `Bundled` priority).
    pub async fn sorted_skills(&self) -> Vec<(SkillManifest, SkillListingMeta)> {
        let skills = self.skills.read().await;
        let mut entries: Vec<(SkillManifest, SkillListingMeta)> = skills
            .values()
            .map(|s| (s.manifest(), s.listing_meta()))
            .collect();
        entries.sort_by(|a, b| a.0.name.cmp(&b.0.name));
        entries
    }

    /// Generates a formatted skill listing string for all builtin skills.
    ///
    /// - Only includes skills where `user_invocable` is `true`
    /// - Sorts by name alphabetically (all builtin skills share
    ///   `SkillSource::Bundled` priority)
    /// - Format matches [`DiskSkillRegistry::generate_listing`]
    pub async fn generate_listing(&self) -> String {
        let entries = self.sorted_skills().await;
        let lines: Vec<String> = entries
            .iter()
            .filter(|(_, meta)| meta.user_invocable)
            .map(|(m, meta)| Self::render_single_listing(m, meta))
            .collect();
        lines.join("\n")
    }

    /// Generates a skill listing **excluding** conditional skills (those
    /// with non-empty `paths`).
    ///
    /// Used as the base for incremental diff computation. Conditional
    /// skills are injected separately via [`find_conditional_matches`].
    pub async fn generate_listing_excluding_conditional(&self) -> String {
        let entries = self.sorted_skills().await;
        let lines: Vec<String> = entries
            .iter()
            .filter(|(_, meta)| meta.user_invocable && meta.paths.is_empty())
            .map(|(m, meta)| Self::render_single_listing(m, meta))
            .collect();
        lines.join("\n")
    }

    /// Find conditional skills whose glob patterns match the given file
    /// paths.
    ///
    /// Returns each matched skill as a [`ConditionalSkillMatch`] with a
    /// rendered listing line including the `⚡ auto-activates on:`
    /// annotation.
    pub async fn find_conditional_matches(
        &self,
        paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        use crate::disk::path_matcher::PathMatcher;

        if paths.is_empty() {
            return Vec::new();
        }
        let entries = self.sorted_skills().await;
        let mut matched = Vec::new();
        for (manifest, meta) in &entries {
            if meta.paths.is_empty() {
                continue;
            }
            let matcher = match PathMatcher::new(&meta.paths) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if paths.iter().any(|p| matcher.matches(p)) {
                matched.push(closeclaw_common::ConditionalSkillMatch {
                    name: manifest.name.clone(),
                    listing_line: Self::render_single_listing(manifest, meta),
                });
            }
        }
        matched
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Skill '{0}' not found")]
    NotFound(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Invalid arguments: {0}")]
    InvalidArgs(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::disk::types::SkillEffort;

    struct MockSkill {
        name: String,
        meta: SkillListingMeta,
    }

    impl MockSkill {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                meta: SkillListingMeta {
                    when_to_use: format!("use {} when needed", name),
                    user_invocable: false,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            }
        }

        fn with_meta(name: &str, meta: SkillListingMeta) -> Self {
            Self {
                name: name.to_string(),
                meta,
            }
        }
    }

    #[async_trait]
    impl Skill for MockSkill {
        fn manifest(&self) -> SkillManifest {
            SkillManifest {
                name: self.name.clone(),
                version: "1.0.0".to_string(),
                description: format!("mock skill {}", self.name),
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

    #[tokio::test]
    async fn test_register_and_get() {
        let registry = BuiltinSkillRegistry::new();
        let skill = Arc::new(MockSkill::new("test_skill"));
        registry.register(skill).await;

        let found = registry.get("test_skill").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().manifest().name, "test_skill");
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let registry = BuiltinSkillRegistry::new();
        let found = registry.get("nonexistent").await;
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_list() {
        let registry = BuiltinSkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("skill_a"))).await;
        registry.register(Arc::new(MockSkill::new("skill_b"))).await;

        let mut names = registry.list().await;
        names.sort();
        assert_eq!(names, vec!["skill_a", "skill_b"]);
    }

    #[tokio::test]
    async fn test_contains() {
        let registry = BuiltinSkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("exists"))).await;

        assert!(registry.contains("exists").await);
        assert!(!registry.contains("missing").await);
    }

    #[tokio::test]
    async fn test_unregister() {
        let registry = BuiltinSkillRegistry::new();
        registry
            .register(Arc::new(MockSkill::new("to_remove")))
            .await;

        assert!(registry.unregister("to_remove").await);
        assert!(!registry.contains("to_remove").await);
        assert!(!registry.unregister("to_remove").await);
    }

    #[tokio::test]
    async fn test_register_replaces() {
        let registry = BuiltinSkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("skill"))).await;
        registry.register(Arc::new(MockSkill::new("skill"))).await;

        let names = registry.list().await;
        assert_eq!(names.len(), 1);
    }

    #[tokio::test]
    async fn test_body_returns_value() {
        let registry = BuiltinSkillRegistry::new();
        registry
            .register(Arc::new(MockSkill::new("body_skill")))
            .await;

        let skill = registry.get("body_skill").await.unwrap();
        assert_eq!(skill.body(), "mock body");
    }

    #[tokio::test]
    async fn test_skill_error_display() {
        let err = SkillError::NotFound("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = SkillError::ExecutionFailed("boom".to_string());
        assert!(err.to_string().contains("boom"));

        let err = SkillError::InvalidArgs("bad".to_string());
        assert!(err.to_string().contains("bad"));
    }

    #[test]
    fn test_skill_manifest_serialization() {
        let manifest = SkillManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            description: "desc".to_string(),
            author: Some("author".to_string()),
            dependencies: vec!["dep1".to_string()],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: SkillManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.author, Some("author".to_string()));
        assert_eq!(parsed.dependencies, vec!["dep1".to_string()]);
    }

    #[test]
    fn test_registry_default() {
        let registry = BuiltinSkillRegistry::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let names = registry.list().await;
            assert!(names.is_empty());
        });
    }

    #[test]
    fn test_default_listing_meta() {
        // A skill that does not override listing_meta() gets the
        // default sentinel: user_invocable = false, everything else empty/default.
        struct NoMetaSkill;
        #[async_trait]
        impl Skill for NoMetaSkill {
            fn manifest(&self) -> SkillManifest {
                SkillManifest {
                    name: "no_meta".into(),
                    version: "0.1".into(),
                    description: "".into(),
                    author: None,
                    dependencies: vec![],
                }
            }
            fn body(&self) -> &str {
                ""
            }
            // listing_meta intentionally omitted — should use default
        }

        let skill = NoMetaSkill;
        let meta = skill.listing_meta();
        assert!(
            !meta.user_invocable,
            "default should be user_invocable: false"
        );
        assert!(meta.when_to_use.is_empty());
        assert!(meta.paths.is_empty());
        assert_eq!(meta.effort, SkillEffort::Unknown);
    }

    #[test]
    fn test_mock_listing_meta() {
        let skill = MockSkill::new("test");
        let meta = skill.listing_meta();
        assert_eq!(meta.when_to_use, "use test when needed");
        assert!(!meta.user_invocable);
        assert!(meta.paths.is_empty());
        assert_eq!(meta.effort, SkillEffort::Unknown);
    }

    #[test]
    fn test_mock_listing_meta_with_meta() {
        let skill = MockSkill::with_meta(
            "custom",
            SkillListingMeta {
                when_to_use: "custom when".into(),
                user_invocable: true,
                paths: vec!["**/*.rs".into()],
                effort: SkillEffort::Large,
            },
        );
        let meta = skill.listing_meta();
        assert_eq!(meta.when_to_use, "custom when");
        assert!(meta.user_invocable);
        assert_eq!(meta.paths, vec!["**/*.rs"]);
        assert_eq!(meta.effort, SkillEffort::Large);
    }

    #[tokio::test]
    async fn test_from_skills_registers_all() {
        let skills: Vec<Arc<dyn Skill>> = vec![
            Arc::new(MockSkill::new("alpha")),
            Arc::new(MockSkill::new("beta")),
        ];
        let registry = BuiltinSkillRegistry::from_skills(skills).await;
        let mut names = registry.list().await;
        names.sort();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert!(registry.contains("alpha").await);
        assert!(registry.contains("beta").await);
    }

    #[tokio::test]
    async fn test_from_skills_empty() {
        let registry = BuiltinSkillRegistry::from_skills(vec![]).await;
        assert!(registry.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_from_skills_overwrites_duplicates() {
        let skills: Vec<Arc<dyn Skill>> = vec![
            Arc::new(MockSkill::new("dup")),
            Arc::new(MockSkill::new("dup")),
        ];
        let registry = BuiltinSkillRegistry::from_skills(skills).await;
        let names = registry.list().await;
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], "dup");
    }

    #[tokio::test]
    async fn test_generate_listing_only_user_invocable() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_meta(
                "visible",
                SkillListingMeta {
                    when_to_use: "when visible".into(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Small,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "hidden",
                SkillListingMeta {
                    when_to_use: "when hidden".into(),
                    user_invocable: false,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
        ])
        .await;
        let listing = registry.generate_listing().await;
        assert!(listing.contains("visible"));
        assert!(!listing.contains("hidden"));
    }

    #[tokio::test]
    async fn test_generate_listing_format_matches_disk() {
        let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_meta(
            "my_skill",
            SkillListingMeta {
                when_to_use: "use when testing".into(),
                user_invocable: true,
                paths: vec![],
                effort: SkillEffort::Medium,
            },
        ))])
        .await;
        let listing = registry.generate_listing().await;
        assert_eq!(
            listing,
            "- **my_skill**: mock skill my_skill — use when testing [effort: medium]"
        );
    }

    #[tokio::test]
    async fn test_generate_listing_sorts_alphabetically() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_meta(
                "zebra",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "alpha",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
        ])
        .await;
        let listing = registry.generate_listing().await;
        let alpha_pos = listing.find("alpha").unwrap();
        let zebra_pos = listing.find("zebra").unwrap();
        assert!(alpha_pos < zebra_pos);
    }

    #[tokio::test]
    async fn test_generate_listing_empty_when_no_invocable() {
        let registry =
            BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::new("hidden"))]).await;
        let listing = registry.generate_listing().await;
        assert!(listing.is_empty());
    }

    #[tokio::test]
    async fn test_generate_listing_excluding_conditional() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_meta(
                "regular",
                SkillListingMeta {
                    when_to_use: "always".into(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "conditional",
                SkillListingMeta {
                    when_to_use: "on match".into(),
                    user_invocable: true,
                    paths: vec!["**/*.rs".into()],
                    effort: SkillEffort::Small,
                },
            )),
        ])
        .await;
        let listing = registry.generate_listing_excluding_conditional().await;
        assert!(listing.contains("regular"));
        assert!(!listing.contains("conditional"));
    }

    #[tokio::test]
    async fn test_find_conditional_matches() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_meta(
                "rust_skill",
                SkillListingMeta {
                    when_to_use: "for rust files".into(),
                    user_invocable: true,
                    paths: vec!["**/*.rs".into()],
                    effort: SkillEffort::Small,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "no_paths",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
        ])
        .await;
        let matches = registry
            .find_conditional_matches(&[std::path::PathBuf::from("src/main.rs")])
            .await;
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "rust_skill");
        assert!(matches[0]
            .listing_line
            .contains("⚡ auto-activates on: **/*.rs"));
    }

    #[tokio::test]
    async fn test_find_conditional_matches_empty_paths() {
        let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_meta(
            "skill",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: true,
                paths: vec!["**/*.rs".into()],
                effort: SkillEffort::Unknown,
            },
        ))])
        .await;
        let matches = registry.find_conditional_matches(&[]).await;
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn test_find_conditional_matches_no_match() {
        let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_meta(
            "skill",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: true,
                paths: vec!["**/*.rs".into()],
                effort: SkillEffort::Unknown,
            },
        ))])
        .await;
        let matches = registry
            .find_conditional_matches(&[std::path::PathBuf::from("file.txt")])
            .await;
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn test_render_single_listing_no_when_to_use() {
        let manifest = SkillManifest {
            name: "bare".into(),
            version: "1.0".into(),
            description: "bare skill".into(),
            author: None,
            dependencies: vec![],
        };
        let meta = SkillListingMeta {
            when_to_use: String::new(),
            user_invocable: true,
            paths: vec![],
            effort: SkillEffort::Unknown,
        };
        let line = BuiltinSkillRegistry::render_single_listing(&manifest, &meta);
        assert_eq!(line, "- **bare**: bare skill");
    }

    #[tokio::test]
    async fn test_render_single_listing_with_paths() {
        let manifest = SkillManifest {
            name: "rs_skill".into(),
            version: "1.0".into(),
            description: "rust skill".into(),
            author: None,
            dependencies: vec![],
        };
        let meta = SkillListingMeta {
            when_to_use: "for rust".into(),
            user_invocable: true,
            paths: vec!["**/*.rs".into(), "**/*.toml".into()],
            effort: SkillEffort::Small,
        };
        let line = BuiltinSkillRegistry::render_single_listing(&manifest, &meta);
        assert_eq!(
            line,
            "- **rs_skill**: rust skill — for rust ⚡ auto-activates on: **/*.rs, **/*.toml [effort: small]"
        );
    }

    #[tokio::test]
    async fn test_user_invocable_names_empty() {
        let registry = BuiltinSkillRegistry::new();
        let names = registry.user_invocable_names().await;
        assert!(names.is_empty());
    }

    #[tokio::test]
    async fn test_user_invocable_names_only_invocable() {
        let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_meta(
            "invocable",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: true,
                paths: vec![],
                effort: SkillEffort::Unknown,
            },
        ))])
        .await;
        let names = registry.user_invocable_names().await;
        assert_eq!(names, vec!["invocable"]);
    }

    #[tokio::test]
    async fn test_user_invocable_names_mixed() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_meta(
                "invocable_a",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "hidden_b",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: false,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "invocable_c",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
        ])
        .await;
        let mut names = registry.user_invocable_names().await;
        names.sort();
        assert_eq!(names, vec!["invocable_a", "invocable_c"]);
    }

    // -----------------------------------------------------------------------
    // listing_entries tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_listing_entries_no_whitelist_and_star() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_meta(
                "alpha",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "beta",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
        ])
        .await;
        // None whitelist — no filtering
        let entries = registry.listing_entries(None, false).await;
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|(_, p)| *p == 4));
        assert!(entries[0].0.contains("alpha"));
        assert!(entries[1].0.contains("beta"));
        // ["*"] should also not filter
        let entries = registry
            .listing_entries(Some(&["*".to_string()]), false)
            .await;
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_listing_entries_with_whitelist() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_meta(
                "alpha",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "beta",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "gamma",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
        ])
        .await;
        let entries = registry
            .listing_entries(Some(&["beta".to_string(), "gamma".to_string()]), false)
            .await;
        assert_eq!(entries.len(), 2);
        assert!(entries[0].0.contains("beta"));
        assert!(entries[1].0.contains("gamma"));
    }

    #[tokio::test]
    async fn test_listing_entries_conditional_and_invocable() {
        let registry = BuiltinSkillRegistry::from_skills(vec![
            Arc::new(MockSkill::with_meta(
                "regular",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "conditional",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: true,
                    paths: vec!["**/*.rs".to_string()],
                    effort: SkillEffort::Unknown,
                },
            )),
            Arc::new(MockSkill::with_meta(
                "hidden",
                SkillListingMeta {
                    when_to_use: String::new(),
                    user_invocable: false,
                    paths: vec![],
                    effort: SkillEffort::Unknown,
                },
            )),
        ])
        .await;
        // exclude_conditional=true: conditional excluded, hidden excluded
        let entries = registry.listing_entries(None, true).await;
        assert_eq!(entries.len(), 1);
        assert!(entries[0].0.contains("regular"));
        assert_eq!(entries[0].1, 4u8);
        // exclude_conditional=false: conditional included, hidden still excluded
        let entries = registry.listing_entries(None, false).await;
        assert_eq!(entries.len(), 2);
    }
}
