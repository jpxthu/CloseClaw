//! DiskSkillRegistry - in-memory registry for disk-loaded skills.

use std::path::Path;

use super::path_matcher::PathMatcher;
use super::types::{DiskSkill, SkillSource};

/// In-memory registry holding all discovered disk skills.
#[derive(Debug, Default)]
pub struct DiskSkillRegistry {
    skills: Vec<DiskSkill>,
}

impl DiskSkillRegistry {
    /// Creates a new registry with the given skills.
    pub fn new(skills: Vec<DiskSkill>) -> Self {
        Self { skills }
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

    /// Returns all skills that originated from the given source.
    pub fn filter_by_source(&self, source: SkillSource) -> Vec<&DiskSkill> {
        self.skills.iter().filter(|s| s.source == source).collect()
    }

    /// Generates a formatted skill listing string for the given agent_id.
    ///
    /// - Sorts by SkillSource priority (Bundled > ExtraDirs > Global > Agent > Project)
    /// - Within the same priority, sorts by name alphabetically
    /// - Filters: only includes skills where `agent_id` is empty or matches the given agent_id
    /// - Format: `- **{name}**: {description}` + optionally ` — {when_to_use}`
    /// - Returns empty string if no skills match
    pub fn generate_listing(&self, agent_id: Option<&str>) -> String {
        let mut filtered: Vec<&DiskSkill> = self
            .skills
            .iter()
            .filter(|s| {
                s.manifest.agent_id.is_empty()
                    || agent_id.map_or(true, |id| s.manifest.agent_id == id)
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
#[path = "registry_tests.rs"]
mod tests;
