//! DiskSkillRegistry - in-memory registry for disk-loaded skills.

use crate::agent::registry::AgentRegistry;
use std::path::Path;
use std::sync::Arc;

use super::path_matcher::PathMatcher;
use super::types::{DiskSkill, SkillSource};

/// In-memory registry holding all discovered disk skills.
#[derive(Debug, Clone)]
pub struct DiskSkillRegistry {
    skills: Vec<DiskSkill>,
    /// Optional reference to the agent configuration registry.
    /// When set, `generate_listing` can look up the skills whitelist
    /// directly from the agent config, avoiding the need for the caller
    /// to pass it explicitly.
    agent_registry: Option<Arc<AgentRegistry>>,
}

impl Default for DiskSkillRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

impl DiskSkillRegistry {
    /// Creates a new registry with the given skills and no agent registry.
    pub fn new(skills: Vec<DiskSkill>) -> Self {
        Self {
            skills,
            agent_registry: None,
        }
    }

    /// Creates an empty registry with no skills and no agent registry.
    fn empty() -> Self {
        Self {
            skills: Vec::new(),
            agent_registry: None,
        }
    }

    /// Inject the agent configuration registry for direct skills-whitelist
    /// lookups.
    pub fn set_agent_registry(&mut self, registry: Arc<AgentRegistry>) {
        self.agent_registry = Some(registry);
    }

    /// Returns skills that have path-based conditional activation (`paths` is non-empty).
    ///
    /// These skills are only auto-activated when the current operation context
    /// involves files matching one of their glob patterns.
    pub fn conditional_skills(&self) -> Vec<&DiskSkill> {
        self.skills
            .iter()
            .filter(|s| !s.manifest.paths.is_empty())
            .collect()
    }

    /// Returns `true` if any path in `paths` matches one of the glob patterns
    /// defined in the named skill's manifest `paths` field.
    ///
    /// Returns `false` when:
    /// - The skill does not exist
    /// - The skill has no paths conditions
    /// - `paths` is empty
    pub fn matches_paths(&self, skill_name: &str, paths: &[&Path]) -> bool {
        if paths.is_empty() {
            return false;
        }
        let skill = match self.get(skill_name) {
            Some(s) => s,
            None => return false,
        };
        if skill.manifest.paths.is_empty() {
            return false;
        }
        let matcher = match PathMatcher::new(&skill.manifest.paths) {
            Ok(m) => m,
            Err(_) => return false,
        };
        paths.iter().any(|p| matcher.matches(p))
    }

    /// Returns a reference to the agent configuration registry, if set.
    pub fn agent_registry(&self) -> Option<&Arc<AgentRegistry>> {
        self.agent_registry.as_ref()
    }

    /// Returns the number of registered skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Returns true if the registry contains no skills.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Looks up a skill by exact name.
    pub fn get(&self, name: &str) -> Option<&DiskSkill> {
        self.skills.iter().find(|s| s.manifest.name == name)
    }

    /// Returns true if a skill with the given name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Returns the names of all registered skills in registration order.
    pub fn list(&self) -> Vec<&str> {
        self.skills
            .iter()
            .map(|s| s.manifest.name.as_str())
            .collect()
    }

    /// Returns all conditional skills (those with non-empty `paths`) that
    /// match any of the given file paths, sorted by [`SkillSource`] priority
    /// (Project > Agent > Global > ExtraDirs > Bundled) with deduplication
    /// by name (higher-priority skill wins).
    ///
    /// Returns an empty vec when `paths` is empty or no conditional skills match.
    pub fn find_matching_skills(&self, paths: &[&Path]) -> Vec<&DiskSkill> {
        if paths.is_empty() {
            return Vec::new();
        }
        let mut matched: Vec<&DiskSkill> = self
            .skills
            .iter()
            .filter(|s| {
                if s.manifest.paths.is_empty() {
                    return false;
                }
                let matcher = match PathMatcher::new(&s.manifest.paths) {
                    Ok(m) => m,
                    Err(_) => return false,
                };
                paths.iter().any(|p| matcher.matches(p))
            })
            .collect();
        matched.sort_by_key(|s| s.source);
        let mut seen = std::collections::HashMap::new();
        matched.retain(|s| seen.insert(s.manifest.name.clone(), s.source).is_none());
        matched
    }

    /// Returns all skills that originated from the given source.
    pub fn filter_by_source(&self, source: SkillSource) -> Vec<&DiskSkill> {
        self.skills.iter().filter(|s| s.source == source).collect()
    }

    /// Generates a formatted skill listing string for the given agent_id.
    ///
    /// - Sorts by SkillSource priority (Project > Agent > Global > ExtraDirs > Bundled)
    /// - Within the same priority, sorts by name alphabetically
    /// - Filters: only includes skills where `agent_id` is empty or matches the given agent_id,
    ///   and `user_invocable` is true (skills with `user_invocable: false` are excluded)
    /// - When `skills_whitelist` is `Some(list)`, only skills whose name appears in
    ///   the list are included (unless the list is `["*"]`, which means no filter).
    /// - When `skills_whitelist` is `None` and an `agent_registry` is set, the whitelist
    ///   is looked up directly from the agent config (direct query path, per design doc).
    /// - Format: `- **{name}**: {description}` + optionally ` — {when_to_use}`
    /// - Returns empty string if no skills match
    pub fn generate_listing(
        &self,
        agent_id: Option<&str>,
        skills_whitelist: Option<&[String]>,
    ) -> String {
        // When no explicit whitelist is provided, attempt to look it up
        // directly from the AgentRegistry (design-doc query path).
        let resolved_whitelist = match skills_whitelist {
            Some(w) => Some(w.to_vec()),
            None => self
                .agent_registry
                .as_ref()
                .and_then(|reg| reg.get(agent_id.unwrap_or("")))
                .and_then(|cfg| {
                    let skills = &cfg.skills;
                    if skills.is_empty() || *skills == ["*"] {
                        None
                    } else {
                        Some(skills.clone())
                    }
                }),
        };
        let resolved_ref = resolved_whitelist.as_deref();
        self.generate_listing_inner(agent_id, resolved_ref)
    }

    /// Generates a skill listing by directly querying the AgentRegistry for
    /// the agent's skills whitelist.
    ///
    /// This is the primary entry point per the design doc: the Skills Registry
    /// queries the AgentRegistry to obtain the skills configuration. Falls back
    /// to showing all skills when the agent registry is not set or the agent
    /// config is not found.
    pub fn generate_listing_for_agent(&self, agent_id: &str) -> String {
        let resolved_whitelist = self
            .agent_registry
            .as_ref()
            .and_then(|reg| reg.get(agent_id))
            .and_then(|cfg| {
                let skills = &cfg.skills;
                if skills.is_empty() || *skills == ["*"] {
                    None
                } else {
                    Some(skills.clone())
                }
            });
        let resolved_ref = resolved_whitelist.as_deref();
        self.generate_listing_inner(Some(agent_id), resolved_ref)
    }

    /// Internal implementation shared by `generate_listing` and
    /// `generate_listing_for_agent`.
    fn generate_listing_inner(
        &self,
        agent_id: Option<&str>,
        skills_whitelist: Option<&[String]>,
    ) -> String {
        let use_whitelist = skills_whitelist
            .filter(|w| !(w.len() == 1 && w[0] == "*"))
            .map(|w| {
                w.iter()
                    .map(|s| s.as_str())
                    .collect::<std::collections::HashSet<_>>()
            });

        let mut filtered: Vec<&DiskSkill> = self
            .skills
            .iter()
            .filter(|s| {
                if !s.manifest.user_invocable {
                    return false;
                }
                if !(s.manifest.agent_id.is_empty()
                    || agent_id.map_or(true, |id| s.manifest.agent_id == id))
                {
                    return false;
                }
                if let Some(ref set) = use_whitelist {
                    set.contains(s.manifest.name.as_str())
                } else {
                    true
                }
            })
            .collect();

        if filtered.is_empty() {
            return String::new();
        }

        // Sort by source priority (lower is higher priority), then by name
        filtered.sort_by(|a, b| {
            let src_cmp = a.source.cmp(&b.source);
            if src_cmp != std::cmp::Ordering::Equal {
                return src_cmp;
            }
            a.manifest.name.cmp(&b.manifest.name)
        });

        let lines: Vec<String> = filtered
            .iter()
            .map(|s| {
                let when = if s.manifest.when_to_use.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", s.manifest.when_to_use)
                };
                let paths_anno = if s.manifest.paths.is_empty() {
                    String::new()
                } else {
                    format!(" ⚡ auto-activates on: {}", s.manifest.paths.join(", "))
                };
                format!(
                    "- **{}**: {}{}{}",
                    s.manifest.name, s.manifest.description, when, paths_anno
                )
            })
            .collect();

        lines.join("\n")
    }
}

#[cfg(test)]
#[path = "registry_tests/mod.rs"]
mod tests;
