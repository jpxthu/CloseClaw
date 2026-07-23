//! DiskSkillRegistry - in-memory registry for disk-loaded skills.

use closeclaw_agent::AgentSkillsQuery;
use closeclaw_common::ConditionalSkillMatch;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::path_matcher::PathMatcher;
use super::types::{DiskSkill, SkillSource};

/// In-memory registry holding all discovered disk skills.
#[derive(Clone)]
pub struct DiskSkillRegistry {
    skills: Vec<DiskSkill>,
    /// Optional reference to the agent skills query trait object.
    /// When set, `generate_listing` can look up the skills whitelist
    /// directly from the agent config, avoiding the need for the caller
    /// to pass it explicitly.
    agent_skills_query: Option<Arc<dyn AgentSkillsQuery>>,
}

impl std::fmt::Debug for DiskSkillRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskSkillRegistry")
            .field("skills", &self.skills)
            .field(
                "agent_skills_query",
                &self
                    .agent_skills_query
                    .as_ref()
                    .map(|_| "<dyn AgentSkillsQuery>"),
            )
            .finish()
    }
}

impl Default for DiskSkillRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

// ---------------------------------------------------------------------------
// Construction & basic accessors
// ---------------------------------------------------------------------------

impl DiskSkillRegistry {
    /// Creates a new registry with the given skills and no agent query.
    pub fn new(skills: Vec<DiskSkill>) -> Self {
        Self {
            skills,
            agent_skills_query: None,
        }
    }

    /// Creates an empty registry with no skills and no agent query.
    fn empty() -> Self {
        Self {
            skills: Vec::new(),
            agent_skills_query: None,
        }
    }

    /// Inject the agent skills query trait object for direct skills-whitelist
    /// lookups.
    pub fn set_agent_skills_query(&mut self, query: Arc<dyn AgentSkillsQuery>) {
        self.agent_skills_query = Some(query);
    }

    /// Returns skills that have path-based conditional activation
    /// (`paths` is non-empty).
    ///
    /// These skills are only auto-activated when the current operation
    /// context involves files matching one of their glob patterns.
    pub fn conditional_skills(&self) -> Vec<&DiskSkill> {
        self.skills
            .iter()
            .filter(|s| !s.manifest.paths.is_empty())
            .collect()
    }

    /// Returns `true` if any path in `paths` matches one of the glob
    /// patterns defined in the named skill's manifest `paths` field.
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

    /// Returns a reference to the agent skills query, if set.
    pub fn agent_skills_query(&self) -> Option<&Arc<dyn AgentSkillsQuery>> {
        self.agent_skills_query.as_ref()
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
}

// ---------------------------------------------------------------------------
// Path matching & source filtering
// ---------------------------------------------------------------------------

impl DiskSkillRegistry {
    /// Returns all conditional skills (those with non-empty `paths`) that
    /// match any of the given file paths, sorted by [`SkillSource`]
    /// priority (Project > Agent > Global > ExtraDirs > Bundled) with
    /// deduplication by name (higher-priority skill wins).
    ///
    /// Returns an empty vec when `paths` is empty or no conditional
    /// skills match.
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
}

// ---------------------------------------------------------------------------
// Listing generation (AgentSkillsQuery path)
// ---------------------------------------------------------------------------

impl DiskSkillRegistry {
    /// Look up the skills whitelist directly from the agent skills query
    /// for the given agent_id.
    ///
    /// Returns `None` when:
    /// - `agent_id` is `None`
    /// - The agent skills query is not set
    /// - The agent config is not found
    /// - The skills list is wildcard (empty or `["*"]`)
    fn lookup_whitelist_from_agent_skills_query(
        &self,
        agent_id: Option<&str>,
    ) -> Option<Vec<String>> {
        let agent_id = agent_id?;
        let query = self.agent_skills_query.as_ref()?;
        query.get_agent_skills(agent_id)
    }

    /// Generates a formatted skill listing string for the given agent_id.
    ///
    /// - Sorts by SkillSource priority
    ///   (Project > Agent > Global > ExtraDirs > Bundled)
    /// - Within the same priority, sorts by name alphabetically
    /// - Filters: only includes skills where `user_invocable` is true
    /// - When `skills_whitelist` is `Some(list)`, only skills whose
    ///   name appears in the list are included (unless the list is
    ///   `["*"]`, which means no filter).
    /// - When `skills_whitelist` is `None` and an `agent_skills_query` is
    ///   set, the whitelist is looked up directly from the agent config
    ///   (direct query path, per design doc).
    /// - Format: `- **{name}**: {description}` + optionally
    ///   ` — {when_to_use}`
    /// - Returns empty string if no skills match
    pub fn generate_listing(
        &self,
        agent_id: Option<&str>,
        skills_whitelist: Option<&[String]>,
    ) -> String {
        let resolved_whitelist = match skills_whitelist {
            Some(w) => Some(w.to_vec()),
            None => self.lookup_whitelist_from_agent_skills_query(agent_id),
        };
        let resolved_ref = resolved_whitelist.as_deref();
        self.generate_listing_inner(resolved_ref)
    }

    /// Generates a skill listing by directly querying the agent skills
    /// query for the agent's skills whitelist.
    ///
    /// This is the primary entry point per the design doc: the Skills
    /// Registry queries the agent config to obtain the skills
    /// configuration. Falls back to showing all skills when the agent
    /// query is not set or the agent config is not found.
    pub fn generate_listing_for_agent(&self, agent_id: &str) -> String {
        let resolved_whitelist = self
            .agent_skills_query
            .as_ref()
            .and_then(|q| q.get_agent_skills(agent_id));
        let resolved_ref = resolved_whitelist.as_deref();
        self.generate_listing_inner(resolved_ref)
    }
}

// ---------------------------------------------------------------------------
// Listing filtering
// ---------------------------------------------------------------------------

impl DiskSkillRegistry {
    /// Generates a skill listing **excluding** conditional skills (those
    /// with non-empty `paths`).
    ///
    /// Used as the base for incremental diff computation. Conditional
    /// skills are injected separately via [`find_conditional_matches`].
    ///
    /// Returns an empty string if no non-conditional skills match.
    pub fn generate_listing_excluding_conditional(
        &self,
        agent_id: Option<&str>,
        skills_whitelist: Option<&[String]>,
    ) -> String {
        let resolved_whitelist = match skills_whitelist {
            Some(w) => Some(w.to_vec()),
            None => self.lookup_whitelist_from_agent_skills_query(agent_id),
        };
        let resolved_ref = resolved_whitelist.as_deref();
        self.generate_listing_inner_excluding_conditional(resolved_ref)
    }

    /// Find conditional skills whose glob patterns match the given file
    /// paths.
    ///
    /// Returns each matched skill as a [`ConditionalSkillMatch`] with a
    /// rendered listing line including the `⚡ auto-activates on:`
    /// annotation.
    pub fn find_conditional_matches(&self, paths: &[PathBuf]) -> Vec<ConditionalSkillMatch> {
        let path_refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();
        self.find_matching_skills(&path_refs)
            .into_iter()
            .map(|skill| {
                let listing_line = Self::render_single_listing(skill);
                ConditionalSkillMatch {
                    name: skill.manifest.name.clone(),
                    listing_line,
                }
            })
            .collect()
    }

    /// Internal implementation that excludes conditional skills from the
    /// listing.
    fn generate_listing_inner_excluding_conditional(
        &self,
        skills_whitelist: Option<&[String]>,
    ) -> String {
        let mut filtered = self.filter_skills_for_listing(skills_whitelist);
        filtered.retain(|s| s.manifest.paths.is_empty());
        if filtered.is_empty() {
            return String::new();
        }
        Self::render_listing(&mut filtered)
    }

    /// Internal implementation shared by `generate_listing` and
    /// `generate_listing_for_agent`.
    fn generate_listing_inner(&self, skills_whitelist: Option<&[String]>) -> String {
        let mut filtered = self.filter_skills_for_listing(skills_whitelist);
        if filtered.is_empty() {
            return String::new();
        }
        Self::render_listing(&mut filtered)
    }

    /// Filter skills by common listing criteria: `user_invocable`
    /// and whitelist membership.
    ///
    /// The caller may apply additional filtering (e.g. excluding
    /// conditional skills) on the returned slice.
    fn filter_skills_for_listing<'a>(
        &'a self,
        skills_whitelist: Option<&[String]>,
    ) -> Vec<&'a DiskSkill> {
        let use_whitelist = skills_whitelist
            .filter(|w| !(w.len() == 1 && w[0] == "*"))
            .map(|w| {
                w.iter()
                    .map(|s| s.as_str())
                    .collect::<std::collections::HashSet<_>>()
            });

        self.skills
            .iter()
            .filter(|s| {
                if !s.manifest.user_invocable {
                    return false;
                }
                // Agent-scoped filtering is handled by directory-based discovery
                // (agents/<id>/skills/), not by manifest fields.
                if let Some(ref set) = use_whitelist {
                    set.contains(s.manifest.name.as_str())
                } else {
                    true
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Listing rendering
// ---------------------------------------------------------------------------

impl DiskSkillRegistry {
    /// Render a single skill's listing line in the same format as
    /// [`render_listing`].
    fn render_single_listing(skill: &DiskSkill) -> String {
        let when = if skill.manifest.when_to_use.is_empty() {
            String::new()
        } else {
            format!(" — {}", skill.manifest.when_to_use)
        };
        let paths_anno = if skill.manifest.paths.is_empty() {
            String::new()
        } else {
            format!(" ⚡ auto-activates on: {}", skill.manifest.paths.join(", "))
        };
        format!(
            "- **{}**: {}{}{}",
            skill.manifest.name, skill.manifest.description, when, paths_anno
        )
    }

    /// Render a pre-filtered, pre-sorted skill slice into a listing
    /// string. Each line is formatted as:
    ///   `- **{name}**: {description} — {when_to_use} ⚡ auto-activates on: {paths}`
    fn render_listing(skills: &mut Vec<&DiskSkill>) -> String {
        skills.sort_by(|a, b| {
            let src_cmp = a.source.cmp(&b.source);
            if src_cmp != std::cmp::Ordering::Equal {
                return src_cmp;
            }
            a.manifest.name.cmp(&b.manifest.name)
        });

        let lines: Vec<String> = skills
            .iter()
            .map(|s| Self::render_single_listing(s))
            .collect();
        lines.join("\n")
    }
}

#[cfg(test)]
#[path = "registry_tests/mod.rs"]
mod tests;
