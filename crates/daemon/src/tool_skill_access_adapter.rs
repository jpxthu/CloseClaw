//! Production tools-side [`ToolDiskSkillAccessAdapter`] /
//! [`ToolBuiltinSkillAccessAdapter`] implementations for the daemon
//! composition root.
//!
//! Thin wrappers over the real `closeclaw_skills` registries for
//! `closeclaw_tools::builtin::SkillTool` (the slash-side equivalents
//! live in [`crate::skill_access_adapter`]): disk bodies are read
//! lazily at invocation time, preserving the registry's
//! rescan/freshness semantics, and execute errors are mapped to the
//! tools-side mirror with `Display` preserved verbatim so
//! `ToolCallError` messages are unchanged.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use closeclaw_skills::{BuiltinSkillRegistry, DiskSkillRegistry, SkillError};
use closeclaw_tools::skill_access::{
    BuiltinSkillAccess, DiskSkillAccess, DiskSkillData, SkillExecuteError,
};

/// Map a skills-side execute error to the tools-side mirror, preserving
/// the payload (and therefore the `Display` output) verbatim.
fn execute_error_to_tools(e: SkillError) -> SkillExecuteError {
    match e {
        SkillError::NotFound(name) => SkillExecuteError::NotFound(name),
        SkillError::ExecutionFailed(msg) => SkillExecuteError::ExecutionFailed(msg),
        SkillError::InvalidArgs(msg) => SkillExecuteError::InvalidArgs(msg),
    }
}

/// Production disk-side adapter wrapping the real disk skill registry.
pub struct ToolDiskSkillAccessAdapter {
    registry: Arc<DiskSkillRegistry>,
}

impl ToolDiskSkillAccessAdapter {
    /// Wrap the given disk skill registry.
    pub fn new(registry: Arc<DiskSkillRegistry>) -> Self {
        Self { registry }
    }
}

impl DiskSkillAccess for ToolDiskSkillAccessAdapter {
    fn load_by_name(&self, name: &str) -> Option<Result<DiskSkillData, std::io::Error>> {
        self.registry.get(name).map(|skill| {
            let skill_dir = skill.skill_dir.clone();
            skill
                .load_body()
                .map(|body| DiskSkillData { body, skill_dir })
        })
    }
}

/// Build the production disk-side port as `Arc<dyn DiskSkillAccess>`.
pub fn tool_disk_skill_access(registry: Arc<DiskSkillRegistry>) -> Arc<dyn DiskSkillAccess> {
    Arc::new(ToolDiskSkillAccessAdapter::new(registry))
}

/// Production builtin-side adapter wrapping the real builtin skill
/// registry.
pub struct ToolBuiltinSkillAccessAdapter {
    registry: Arc<BuiltinSkillRegistry>,
}

impl ToolBuiltinSkillAccessAdapter {
    /// Wrap the given builtin skill registry.
    pub fn new(registry: Arc<BuiltinSkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl BuiltinSkillAccess for ToolBuiltinSkillAccessAdapter {
    async fn execute_by_name(
        &self,
        name: &str,
        args: Option<Value>,
    ) -> Option<Result<String, SkillExecuteError>> {
        let skill = self.registry.get(name).await?;
        Some(skill.execute(args).await.map_err(execute_error_to_tools))
    }
}

/// Build the production builtin-side port as
/// `Arc<dyn BuiltinSkillAccess>`.
pub fn tool_builtin_skill_access(
    registry: Arc<BuiltinSkillRegistry>,
) -> Arc<dyn BuiltinSkillAccess> {
    Arc::new(ToolBuiltinSkillAccessAdapter::new(registry))
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

    struct EchoSkill;

    #[async_trait]
    impl Skill for EchoSkill {
        fn manifest(&self) -> SkillManifest {
            mock_manifest("echo")
        }
        fn body(&self) -> &str {
            "echo body"
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
            let tools_err = execute_error_to_tools(skills_err);
            assert_eq!(tools_err.to_string(), skills_display);
            assert_eq!(tools_err.to_string(), expected_display);
        }
    }

    // ── Disk adapter against the real registry ───────────────────────

    #[test]
    fn disk_adapter_loads_body_with_dir_and_returns_none_for_missing_skill() {
        let tmp = tempfile::TempDir::new().unwrap();
        let readme = tmp.path().join("SKILL.md");
        std::fs::write(&readme, "---\ndescription: test\n---\n\n# Hello\n").unwrap();
        let registry = Arc::new(DiskSkillRegistry::new(vec![make_disk_skill(
            "alpha",
            readme,
            tmp.path().to_path_buf(),
        )]));
        let adapter = ToolDiskSkillAccessAdapter::new(Arc::clone(&registry));

        let loaded = adapter.load_by_name("alpha").expect("skill exists");
        let data = loaded.expect("body loads");
        assert_eq!(data.body, "# Hello");
        assert_eq!(data.skill_dir, tmp.path());

        // Missing skill → None (caller falls through to the builtin side).
        assert!(adapter.load_by_name("missing").is_none());
    }

    // ── Builtin adapter against the real registry ────────────────────

    #[tokio::test]
    async fn builtin_adapter_executes_and_maps_errors() {
        let registry = Arc::new(BuiltinSkillRegistry::new());
        registry.register(Arc::new(EchoSkill)).await;

        let adapter = ToolBuiltinSkillAccessAdapter::new(Arc::clone(&registry));

        // Existing skill with failing execute → Some(Err) mapped to the
        // tools mirror, payload and Display preserved.
        match adapter.execute_by_name("echo", None).await {
            Some(Err(SkillExecuteError::ExecutionFailed(msg))) => assert_eq!(msg, "boom"),
            other => panic!("expected mapped execution failure, got {other:?}"),
        }

        // Missing skill → None (caller reports ToolCallError::NotFound).
        assert!(adapter.execute_by_name("missing", None).await.is_none());
    }

    #[tokio::test]
    async fn factories_return_trait_objects() {
        let disk: Arc<dyn DiskSkillAccess> =
            tool_disk_skill_access(Arc::new(DiskSkillRegistry::new(vec![])));
        assert!(disk.load_by_name("missing").is_none());

        let builtin: Arc<dyn BuiltinSkillAccess> =
            tool_builtin_skill_access(Arc::new(BuiltinSkillRegistry::new()));
        assert!(builtin.execute_by_name("missing", None).await.is_none());
    }
}
