//! Production [`DiskSkillAccess`] / [`BuiltinSkillAccess`]
//! implementations for the daemon composition root.
//!
//! The adapters are thin wrappers over the real `closeclaw_skills`
//! registries: [`DiskSkillAccessAdapter`] wraps a [`DiskSkillRegistry`]
//! snapshot (bodies are read lazily at invocation time, preserving the
//! registry's rescan/freshness semantics), and
//! [`BuiltinSkillAccessAdapter`] wraps [`BuiltinSkillRegistry`].
//! Execute errors are mapped to the slash-side mirror with `Display`
//! preserved verbatim so reply texts are unchanged.

use std::sync::Arc;

use async_trait::async_trait;

use closeclaw_skills::{BuiltinSkillRegistry, DiskSkillRegistry, SkillError};
use closeclaw_slash::skill_access::{
    BuiltinSkillAccess, DiskSkillAccess, DiskSkillBody, SkillExecuteError,
};

/// Map a skills-side execute error to the slash-side mirror, preserving
/// the payload (and therefore the `Display` output) verbatim.
fn execute_error_to_slash(e: SkillError) -> SkillExecuteError {
    match e {
        SkillError::NotFound(name) => SkillExecuteError::NotFound(name),
        SkillError::ExecutionFailed(msg) => SkillExecuteError::ExecutionFailed(msg),
        SkillError::InvalidArgs(msg) => SkillExecuteError::InvalidArgs(msg),
    }
}

/// Production disk-side adapter wrapping the real disk skill registry.
pub struct DiskSkillAccessAdapter {
    registry: Arc<DiskSkillRegistry>,
}

impl DiskSkillAccessAdapter {
    /// Wrap the given disk skill registry.
    pub fn new(registry: Arc<DiskSkillRegistry>) -> Self {
        Self { registry }
    }
}

impl DiskSkillAccess for DiskSkillAccessAdapter {
    fn user_invocable_names(&self) -> Vec<String> {
        self.registry.user_invocable_names()
    }

    fn load_by_name(&self, name: &str) -> Option<Result<DiskSkillBody, std::io::Error>> {
        self.registry.get(name).map(|skill| {
            let skill_dir = skill.skill_dir.clone();
            skill
                .load_body()
                .map(|body| DiskSkillBody { body, skill_dir })
        })
    }
}

/// Build the production disk-side port as `Arc<dyn DiskSkillAccess>`.
pub fn disk_skill_access(registry: Arc<DiskSkillRegistry>) -> Arc<dyn DiskSkillAccess> {
    Arc::new(DiskSkillAccessAdapter::new(registry))
}

/// Production builtin-side adapter wrapping the real builtin skill
/// registry.
pub struct BuiltinSkillAccessAdapter {
    registry: Arc<BuiltinSkillRegistry>,
}

impl BuiltinSkillAccessAdapter {
    /// Wrap the given builtin skill registry.
    pub fn new(registry: Arc<BuiltinSkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl BuiltinSkillAccess for BuiltinSkillAccessAdapter {
    async fn user_invocable_names(&self) -> Vec<String> {
        self.registry.user_invocable_names().await
    }

    async fn execute_by_name(&self, name: &str) -> Option<Result<String, SkillExecuteError>> {
        let skill = self.registry.get(name).await?;
        Some(skill.execute(None).await.map_err(execute_error_to_slash))
    }
}

/// Build the production builtin-side port as
/// `Arc<dyn BuiltinSkillAccess>`.
pub fn builtin_skill_access(registry: Arc<BuiltinSkillRegistry>) -> Arc<dyn BuiltinSkillAccess> {
    Arc::new(BuiltinSkillAccessAdapter::new(registry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_skills::disk::types::{
        DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
    };
    use closeclaw_skills::Skill;

    fn make_disk_skill(
        name: &str,
        readme_path: std::path::PathBuf,
        skill_dir: std::path::PathBuf,
    ) -> DiskSkill {
        DiskSkill {
            source: SkillSource::Global,
            manifest: SkillManifest {
                name: name.into(),
                description: format!("test skill {name}"),
                when_to_use: String::new(),
                context: SkillContext::Inline,
                effort: SkillEffort::Small,
                paths: vec![],
                user_invocable: true,
            },
            readme_path,
            skill_dir,
        }
    }

    fn mock_manifest(name: &str) -> SkillManifest {
        SkillManifest {
            name: name.into(),
            description: "mock".into(),
            when_to_use: String::new(),
            context: SkillContext::Inline,
            effort: SkillEffort::Small,
            paths: vec![],
            user_invocable: true,
        }
    }

    struct FailingSkill;

    #[async_trait]
    impl Skill for FailingSkill {
        fn manifest(&self) -> SkillManifest {
            mock_manifest("failing")
        }
        fn body(&self) -> &str {
            "failing body"
        }
        async fn execute(&self, _args: Option<serde_json::Value>) -> Result<String, SkillError> {
            Err(SkillError::ExecutionFailed("boom".to_string()))
        }
    }

    // ── Error mapping（错误路径） ─────────────────────────────────────

    #[test]
    fn execute_error_mapping_preserves_payload_and_display_for_all_variants() {
        let cases = [
            (SkillError::NotFound("x".into()), "Skill 'x' not found"),
            (
                SkillError::ExecutionFailed("boom".into()),
                "Execution failed: boom",
            ),
            (
                SkillError::InvalidArgs("bad".into()),
                "Invalid arguments: bad",
            ),
        ];
        for (skills_err, expected_display) in cases {
            // Capture the skills-side Display before the error is moved.
            let skills_display = skills_err.to_string();
            let slash_err = execute_error_to_slash(skills_err);
            assert_eq!(slash_err.to_string(), skills_display);
            assert_eq!(slash_err.to_string(), expected_display);
        }
    }

    // ── Disk adapter against the real registry ───────────────────────

    #[test]
    fn disk_adapter_lists_names_loads_body_and_returns_none_for_missing_skill() {
        let tmp = tempfile::TempDir::new().unwrap();
        let readme = tmp.path().join("SKILL.md");
        std::fs::write(&readme, "---\ndescription: test\n---\n\n# Hello\n").unwrap();
        let registry = Arc::new(DiskSkillRegistry::new(vec![make_disk_skill(
            "alpha",
            readme,
            tmp.path().to_path_buf(),
        )]));
        let adapter = DiskSkillAccessAdapter::new(Arc::clone(&registry));

        assert_eq!(adapter.user_invocable_names(), vec!["alpha"]);

        let loaded = adapter.load_by_name("alpha").expect("skill exists");
        let body = loaded.expect("body loads");
        assert_eq!(body.body, "# Hello");
        assert_eq!(body.skill_dir, tmp.path());

        // Missing skill → None (caller falls through to the builtin side).
        assert!(adapter.load_by_name("missing").is_none());
    }

    #[test]
    fn disk_adapter_surfaces_body_read_failure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let readme = tmp.path().join("SKILL.md");
        std::fs::write(&readme, "---\ndescription: test\n---\n\n# Body").unwrap();
        let registry = Arc::new(DiskSkillRegistry::new(vec![make_disk_skill(
            "vanishing",
            readme.clone(),
            tmp.path().to_path_buf(),
        )]));
        let adapter = DiskSkillAccessAdapter::new(registry);

        // Delete the file after registration to simulate load failure.
        std::fs::remove_file(&readme).unwrap();

        match adapter.load_by_name("vanishing") {
            Some(Err(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected body read failure, got {other:?}"),
        }
    }

    // ── Builtin adapter against the real registry ────────────────────

    #[tokio::test]
    async fn builtin_adapter_lists_names_executes_and_maps_errors() {
        let registry = Arc::new(BuiltinSkillRegistry::new());
        registry.register(Arc::new(FailingSkill)).await;

        let adapter = BuiltinSkillAccessAdapter::new(Arc::clone(&registry));
        assert_eq!(adapter.user_invocable_names().await, vec!["failing"]);

        // Existing skill with failing execute → Some(Err) mapped to the
        // slash mirror, payload and Display preserved.
        match adapter.execute_by_name("failing").await {
            Some(Err(SkillExecuteError::ExecutionFailed(msg))) => assert_eq!(msg, "boom"),
            other => panic!("expected mapped execution failure, got {other:?}"),
        }

        // Missing skill → None (caller falls through to 未知技能).
        assert!(adapter.execute_by_name("missing").await.is_none());
    }

    #[tokio::test]
    async fn factories_return_trait_objects() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let disk: Arc<dyn DiskSkillAccess> = disk_skill_access(registry);
        assert!(disk.user_invocable_names().is_empty());

        let builtin: Arc<dyn BuiltinSkillAccess> =
            builtin_skill_access(Arc::new(BuiltinSkillRegistry::new()));
        assert!(builtin.user_invocable_names().await.is_empty());
        assert!(builtin.execute_by_name("missing").await.is_none());
    }
}
