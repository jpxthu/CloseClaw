//! DiskSkillRegistry - in-memory registry for disk-loaded skills.

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
                format!(
                    "- **{}**: {}{}",
                    s.manifest.name, s.manifest.description, when
                )
            })
            .collect();

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{SkillContext, SkillEffort, SkillManifest, SkillSource};
    use super::DiskSkill;
    use super::DiskSkillRegistry;
    use std::path::PathBuf;

    fn skill(name: &str, source: SkillSource) -> DiskSkill {
        DiskSkill {
            source,
            manifest: SkillManifest {
                name: name.into(),
                description: format!("desc of {}", name),
                allowed_tools: vec![],
                when_to_use: String::new(),
                context: SkillContext::default(),
                agent: String::new(),
                agent_id: String::new(),
                effort: SkillEffort::default(),
                paths: vec![],
                user_invocable: false,
            },
            readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
            skill_dir: PathBuf::from(format!("/skills/{}", name)),
        }
    }

    #[test]
    fn test_new_and_len() {
        let r = DiskSkillRegistry::new(vec![skill("a", SkillSource::Bundled)]);
        assert_eq!(r.len(), 1);
        assert!(!r.is_empty());
    }

    #[test]
    fn test_empty_registry() {
        let r = DiskSkillRegistry::new(vec![]);
        assert!(r.is_empty());
        assert!(r.get("any").is_none());
        assert!(!r.contains("any"));
        assert!(r.list().is_empty());
        assert!(r.filter_by_source(SkillSource::Bundled).is_empty());
    }

    #[test]
    fn test_get() {
        let r = DiskSkillRegistry::new(vec![
            skill("foo", SkillSource::Agent),
            skill("bar", SkillSource::Project),
        ]);
        assert_eq!(r.get("foo").unwrap().manifest.name, "foo");
        assert!(r.get("baz").is_none());
    }

    #[test]
    fn test_contains() {
        let r = DiskSkillRegistry::new(vec![skill("x", SkillSource::Global)]);
        assert!(r.contains("x"));
        assert!(!r.contains("y"));
    }

    #[test]
    fn test_list() {
        let r = DiskSkillRegistry::new(vec![
            skill("z", SkillSource::Bundled),
            skill("a", SkillSource::Global),
        ]);
        assert_eq!(r.list(), vec!["z", "a"]);
    }

    #[test]
    fn test_filter_by_source() {
        let r = DiskSkillRegistry::new(vec![
            skill("b1", SkillSource::Bundled),
            skill("g1", SkillSource::Global),
            skill("b2", SkillSource::Bundled),
        ]);
        assert_eq!(r.filter_by_source(SkillSource::Bundled).len(), 2);
        assert_eq!(r.filter_by_source(SkillSource::Global).len(), 1);
        assert_eq!(r.filter_by_source(SkillSource::Agent).len(), 0);
    }

    fn skill_with_agent_id(name: &str, source: SkillSource, agent_id: &str) -> DiskSkill {
        DiskSkill {
            source,
            manifest: SkillManifest {
                name: name.into(),
                description: format!("desc of {}", name),
                allowed_tools: vec![],
                when_to_use: String::new(),
                context: SkillContext::default(),
                agent: String::new(),
                agent_id: agent_id.into(),
                effort: SkillEffort::default(),
                paths: vec![],
                user_invocable: false,
            },
            readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
            skill_dir: PathBuf::from(format!("/skills/{}", name)),
        }
    }

    fn skill_with_when_to_use(name: &str, source: SkillSource, when_to_use: &str) -> DiskSkill {
        DiskSkill {
            source,
            manifest: SkillManifest {
                name: name.into(),
                description: format!("desc of {}", name),
                allowed_tools: vec![],
                when_to_use: when_to_use.into(),
                context: SkillContext::default(),
                agent: String::new(),
                agent_id: String::new(),
                effort: SkillEffort::default(),
                paths: vec![],
                user_invocable: false,
            },
            readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
            skill_dir: PathBuf::from(format!("/skills/{}", name)),
        }
    }

    #[test]
    fn test_generate_listing_empty() {
        let r = DiskSkillRegistry::new(vec![]);
        assert_eq!(r.generate_listing(None), "");
        assert_eq!(r.generate_listing(Some("agent1")), "");
    }

    #[test]
    fn test_generate_listing_single() {
        let r = DiskSkillRegistry::new(vec![skill("foo", SkillSource::Bundled)]);
        let listing = r.generate_listing(None);
        assert!(listing.contains("**foo**"));
        assert!(listing.contains("desc of foo"));
    }

    #[test]
    fn test_generate_listing_sorted_by_priority_and_name() {
        let r = DiskSkillRegistry::new(vec![
            skill("z_bundled", SkillSource::Bundled),
            skill("a_bundled", SkillSource::Bundled),
            skill("z_global", SkillSource::Global),
            skill("a_global", SkillSource::Global),
            skill("z_agent", SkillSource::Agent),
            skill("a_agent", SkillSource::Agent),
        ]);
        let listing = r.generate_listing(None);
        let lines: Vec<&str> = listing.lines().collect();
        assert_eq!(lines.len(), 6);
        // Bundled before Global before Agent
        assert!(listing.find("**a_bundled**").unwrap() < listing.find("**a_global**").unwrap());
        assert!(listing.find("**a_global**").unwrap() < listing.find("**a_agent**").unwrap());
        // Within Bundled, alphabetical order
        assert!(listing.find("**a_bundled**").unwrap() < listing.find("**z_bundled**").unwrap());
    }

    #[test]
    fn test_generate_listing_agent_id_filter() {
        let r = DiskSkillRegistry::new(vec![
            skill_with_agent_id("skill_a", SkillSource::Agent, "agent1"),
            skill_with_agent_id("skill_b", SkillSource::Agent, "agent2"),
            skill_with_agent_id("skill_c", SkillSource::Agent, ""), // no restriction
        ]);
        // None = no filter, all 3 returned
        assert_eq!(r.generate_listing(None).lines().count(), 3);
        // agent1 filter = skill_a (matches) + skill_c (no restriction)
        let listing = r.generate_listing(Some("agent1"));
        assert!(listing.contains("**skill_a**"));
        assert!(listing.contains("**skill_c**"));
        assert!(!listing.contains("**skill_b**"));
    }

    #[test]
    fn test_generate_listing_when_to_use() {
        let r = DiskSkillRegistry::new(vec![
            skill_with_when_to_use("foo", SkillSource::Bundled, "Use when you need foo"),
            skill("bar", SkillSource::Bundled), // no when_to_use
        ]);
        let listing = r.generate_listing(None);
        assert!(listing.contains(" — Use when you need foo"));
        // bar should NOT have the dash separator
        let bar_line = listing.lines().find(|l| l.contains("**bar**")).unwrap();
        assert!(!bar_line.contains(" — "));
    }
}
