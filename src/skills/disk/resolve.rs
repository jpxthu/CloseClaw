//! Skill resolution - routes skill lookup between disk and bundled registries.

use super::types::DiskSkill;
use super::DiskSkillRegistry;
use crate::skills::registry::{Skill, SkillRegistry};
use std::sync::Arc;

/// Result of a skill resolution attempt.
pub enum ResolvedSkill<'a> {
    /// Skill loaded from disk.
    Disk(&'a DiskSkill),
    /// Skill from the bundled registry.
    Bundled(Arc<dyn Skill>),
}

/// Resolves a skill by name, checking disk registry first, then bundled registry.
///
/// Resolution order:
/// 1. Disk registry (synchronous, checked first)
/// 2. Bundled registry (async, checked second)
///
/// Returns `None` if the skill is not found in either registry.
pub async fn resolve_skill<'a>(
    name: &str,
    disk_registry: &'a DiskSkillRegistry,
    skill_registry: &'a SkillRegistry,
) -> Option<ResolvedSkill<'a>> {
    // Check disk registry first (synchronous)
    if let Some(skill) = disk_registry.get(name) {
        return Some(ResolvedSkill::Disk(skill));
    }
    // Then check bundled registry (async)
    if let Some(skill) = skill_registry.get(name).await {
        return Some(ResolvedSkill::Bundled(skill));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{resolve_skill, DiskSkillRegistry, ResolvedSkill};
    use crate::skills::registry::{Skill, SkillManifest, SkillRegistry};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct BundledMockSkill(String);

    #[async_trait]
    impl Skill for BundledMockSkill {
        fn manifest(&self) -> SkillManifest {
            SkillManifest {
                name: self.0.clone(),
                version: "1.0".into(),
                description: "bundled".into(),
                author: None,
                dependencies: vec![],
            }
        }
        fn methods(&self) -> Vec<&str> {
            vec![]
        }
        async fn execute(
            &self,
            _method: &str,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, crate::skills::registry::SkillError> {
            Ok(serde_json::Value::Null)
        }
    }

    fn make_disk_skill(name: &str) -> super::super::types::DiskSkill {
        super::super::types::DiskSkill {
            source: super::super::types::SkillSource::Global,
            manifest: super::super::types::SkillManifest {
                name: name.into(),
                description: format!("desc of {}", name),
                allowed_tools: vec![],
                when_to_use: String::new(),
                context: super::super::types::SkillContext::default(),
                agent: String::new(),
                agent_id: String::new(),
                effort: super::super::types::SkillEffort::default(),
                paths: vec![],
                user_invocable: false,
            },
            readme_path: std::path::PathBuf::from("/tmp/test"),
            skill_dir: std::path::PathBuf::from("/tmp/test"),
        }
    }

    #[tokio::test]
    async fn test_resolve_disk_hit() {
        let disk_reg = DiskSkillRegistry::new(vec![make_disk_skill("disk-skill")]);
        let mut bundled_reg = SkillRegistry::new();
        bundled_reg
            .register(Arc::new(BundledMockSkill("disk-skill".into())))
            .await;

        let result = resolve_skill("disk-skill", &disk_reg, &bundled_reg).await;
        match result {
            Some(ResolvedSkill::Disk(_)) => {}
            other => panic!("expected Disk, got unexpected variant"),
        }
    }

    #[tokio::test]
    async fn test_resolve_bundled_hit() {
        let disk_reg = DiskSkillRegistry::new(vec![]);
        let mut bundled_reg = SkillRegistry::new();
        bundled_reg
            .register(Arc::new(BundledMockSkill("bundled-skill".into())))
            .await;

        let result = resolve_skill("bundled-skill", &disk_reg, &bundled_reg).await;
        match result {
            Some(ResolvedSkill::Bundled(_)) => {}
            other => panic!("expected Bundled, got unexpected variant"),
        }
    }

    #[tokio::test]
    async fn test_resolve_not_found() {
        let disk_reg = DiskSkillRegistry::new(vec![]);
        let bundled_reg = SkillRegistry::new();

        let result = resolve_skill("nonexistent", &disk_reg, &bundled_reg).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_resolve_disk_priority_over_bundled() {
        // Both disk and bundled have a skill named "foo"; disk should win
        let disk_reg = DiskSkillRegistry::new(vec![make_disk_skill("foo")]);
        let mut bundled_reg = SkillRegistry::new();
        bundled_reg
            .register(Arc::new(BundledMockSkill("foo".into())))
            .await;

        let result = resolve_skill("foo", &disk_reg, &bundled_reg).await;
        match result {
            Some(ResolvedSkill::Disk(_)) => {}
            other => panic!("expected Disk (priority), got unexpected variant"),
        }
    }
}
