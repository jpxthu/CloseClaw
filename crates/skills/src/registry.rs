//! Skill Registry - manages skill registration and discovery

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

// Re-export the unified manifest type so downstream modules can
// continue importing from this crate root.
pub use crate::disk::types::{SkillManifest, SkillSource};

/// Skill trait - implemented by each skill
#[async_trait]
pub trait Skill: Send + Sync {
    /// Get the shared skill manifest.
    ///
    /// Both disk-based and bundled skills return the same
    /// [`crate::disk::types::SkillManifest`] type, ensuring a
    /// single source of truth for skill metadata.
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

/// Render a single skill's listing line from a shared [`SkillManifest`].
///
/// Both disk-based and builtin registries delegate to this function
/// to avoid duplicated rendering logic.
pub(crate) fn render_skill_listing(manifest: &SkillManifest) -> String {
    let when = if manifest.when_to_use.is_empty() {
        String::new()
    } else {
        format!(" — {}", manifest.when_to_use)
    };
    let paths_anno = if manifest.paths.is_empty() {
        String::new()
    } else {
        format!(" ⚡ auto-activates on: {}", manifest.paths.join(", "))
    };
    let effort_anno = match manifest.effort {
        crate::disk::types::SkillEffort::Unknown => String::new(),
        effort => format!(" [effort: {}]", effort),
    };
    format!(
        "- **{}**: {}{}{}{}",
        manifest.name, manifest.description, when, paths_anno, effort_anno,
    )
}

/// Convert an optional whitelist slice to a `HashSet` for O(1) lookups.
/// Treats `["*"]` and empty as `None` (no filter).
pub(crate) fn resolve_whitelist_set(
    whitelist: Option<&[String]>,
) -> Option<std::collections::HashSet<&str>> {
    whitelist
        .filter(|w| !(w.len() == 1 && w[0] == "*"))
        .map(|w| w.iter().map(|s| s.as_str()).collect())
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

    /// Return structured listing entries `(name, source, line)` suitable
    /// for merge into the combined listing in bridge.rs.
    ///
    /// All builtin skills have source [`SkillSource::Bundled`].
    /// When `exclude_conditional` is `true`, skills with non-empty
    /// `paths` are excluded (unless their name appears in `activated`).
    pub async fn listing_entries_with_names(
        &self,
        skills_whitelist: Option<&[String]>,
        exclude_conditional: bool,
        activated: Option<&[String]>,
    ) -> Vec<(String, SkillSource, String)> {
        let entries = self.sorted_skills().await;
        let use_whitelist = resolve_whitelist_set(skills_whitelist);
        let activated_set: std::collections::HashSet<&str> = activated
            .map(|a| a.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();

        let mut filtered: Vec<(String, SkillSource, String)> = entries
            .into_iter()
            .filter(|m| {
                let conditional_active =
                    !m.paths.is_empty() && activated_set.contains(m.name.as_str());
                let passes_user_invocable = m.user_invocable || conditional_active;
                let passes_conditional =
                    !exclude_conditional || m.paths.is_empty() || conditional_active;
                let passes_whitelist = match &use_whitelist {
                    Some(set) => set.contains(m.name.as_str()),
                    None => true,
                };
                passes_user_invocable && passes_conditional && passes_whitelist
            })
            .map(|m| {
                let name = m.name.clone();
                let line = Self::render_single_listing(&m);
                (name, SkillSource::Bundled, line)
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
            .filter(|m| m.user_invocable)
            .map(|m| m.name.clone())
            .collect()
    }

    /// Render a single builtin skill's listing line.
    ///
    /// Delegates to the shared [`render_skill_listing`] function.
    pub fn render_single_listing(manifest: &crate::disk::types::SkillManifest) -> String {
        render_skill_listing(manifest)
    }

    /// Collects all skills with their metadata, sorted by name
    /// (all builtin skills share the same `Bundled` priority).
    pub async fn sorted_skills(&self) -> Vec<crate::disk::types::SkillManifest> {
        let skills = self.skills.read().await;
        let mut entries: Vec<crate::disk::types::SkillManifest> =
            skills.values().map(|s| s.manifest()).collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
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
            .filter(|m| m.user_invocable)
            .map(Self::render_single_listing)
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
            .filter(|m| m.user_invocable && m.paths.is_empty())
            .map(Self::render_single_listing)
            .collect();
        lines.join("\n")
    }

    /// Generate a skill listing that includes both the base (non-conditional,
    /// user-invocable) skills and any conditional skills whose names appear in
    /// `activated`.
    ///
    /// Activated conditional skills are included **regardless** of their
    /// `user_invocable` declaration (activation overrides the filter).
    /// Non-conditional skills are still filtered by `user_invocable` as usual.
    ///
    /// Builtin skills do not currently define conditional activation paths
    /// in practice, so this method falls back to the non-conditional listing.
    /// If builtin manifests gain `paths` support in the future, this method
    /// will need updating to apply the same activation-aware logic.
    pub async fn generate_listing_with_activated(&self, activated: &[String]) -> String {
        let activated_set: std::collections::HashSet<&str> =
            activated.iter().map(|s| s.as_str()).collect();
        let entries = self.sorted_skills().await;
        let lines: Vec<String> = entries
            .iter()
            .filter(|m| {
                if m.paths.is_empty() {
                    m.user_invocable
                } else {
                    activated_set.contains(m.name.as_str())
                }
            })
            .map(Self::render_single_listing)
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
        for manifest in &entries {
            if manifest.paths.is_empty() {
                continue;
            }
            let matcher = match PathMatcher::new(&manifest.paths) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if paths.iter().any(|p| matcher.matches(p)) {
                matched.push(closeclaw_common::ConditionalSkillMatch {
                    name: manifest.name.clone(),
                    listing_line: Self::render_single_listing(manifest),
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

#[path = "registry_unit_tests.rs"]
#[cfg(test)]
mod tests;

#[path = "registry_with_activated_tests.rs"]
#[cfg(test)]
mod with_activated_tests;
