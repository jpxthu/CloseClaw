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
}
