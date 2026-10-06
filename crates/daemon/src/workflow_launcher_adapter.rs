//! Production [`WorkflowLauncher`] implementation for the daemon
//! composition root.
//!
//! [`EngineWorkflowLauncher`] is a stateless adapter assembling the
//! `closeclaw_workflow` three-piece set — definition loader, engine,
//! and message builders (context append + Step 0 goal message). It
//! caches no state, so a single instance can be shared by every
//! `/workflow` invocation. Load errors are mapped to the slash-side
//! mirror with `Display` preserved verbatim so reply texts are
//! unchanged (`工作流 "{name}" 加载失败：{e}`).

use std::path::Path;
use std::sync::Arc;

use closeclaw_slash::workflow_launcher::{WorkflowLaunch, WorkflowLauncher, WorkflowLoadError};
use closeclaw_workflow::context_append::build_workflow_context_append;
use closeclaw_workflow::definition::build_goal_message;
use closeclaw_workflow::definition_loader::WorkflowDefinitionLoader;
use closeclaw_workflow::engine::WorkflowEngine;
use closeclaw_workflow::error::WorkflowError;
use closeclaw_workflow::run::GoalHint;

/// Map a workflow-side error to the slash-side mirror, preserving the
/// payload (and therefore the `Display` output) verbatim.
fn load_error_to_slash(e: WorkflowError) -> WorkflowLoadError {
    match e {
        WorkflowError::InvalidDefinition(m) => WorkflowLoadError::InvalidDefinition(m),
        WorkflowError::ParseError(m) => WorkflowLoadError::ParseError(m),
        WorkflowError::DefinitionNotFound(n) => WorkflowLoadError::DefinitionNotFound(n),
        other => WorkflowLoadError::Other(other.to_string()),
    }
}

/// Stateless production adapter wrapping the real workflow loader,
/// engine, and message builders.
pub struct EngineWorkflowLauncher;

/// Build the production workflow launcher as `Arc<dyn WorkflowLauncher>`.
pub fn engine_workflow_launcher() -> Arc<dyn WorkflowLauncher> {
    Arc::new(EngineWorkflowLauncher)
}

impl WorkflowLauncher for EngineWorkflowLauncher {
    fn start(
        &self,
        name: &str,
        agent_workspace: Option<&Path>,
        global_workflows: Option<&Path>,
    ) -> Result<WorkflowLaunch, WorkflowLoadError> {
        let workflow = WorkflowDefinitionLoader::load(name, agent_workspace, global_workflows)
            .map_err(load_error_to_slash)?;
        let run = WorkflowEngine::start(&workflow);
        let goal_message = build_goal_message(&workflow.steps[0], GoalHint::Normal);
        let context_append = build_workflow_context_append(&workflow);
        let first_step_name = workflow.steps[0].name.clone();
        Ok(WorkflowLaunch {
            context_append,
            goal_message,
            first_step_name,
            // The erased handle carries the serialized run (checkpoint) form:
            // neither slash nor gateway names workflow types on this path.
            run: Box::new(serde_json::to_value(&run).expect("workflow run must serialize")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_workflow::run::WorkflowRun;

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Write a single-step workflow SKILL.md with a complete transition.
    fn write_workflow_file(dir: &Path, name: &str) {
        let wf_dir = dir.join("workflows").join(name);
        std::fs::create_dir_all(&wf_dir).unwrap();
        let yaml = concat!(
            "id: test-wf\n",
            "name: Test Workflow\n",
            "description: A test workflow\n",
            "steps:\n",
            "  - id: 0\n",
            "    name: Step Zero\n",
            "    goal: Do the first thing\n",
            "    verify:\n",
            "      - Check output\n",
            "    transitions:\n",
            "      - action: complete\n",
        );
        let content = format!("---\n{yaml}\n---\n\nBody.\n");
        std::fs::write(wf_dir.join("SKILL.md"), content).unwrap();
    }

    // ── Message builders（migrated from the deleted slash wrapper） ────

    /// Equivalent coverage of the former
    /// `WorkflowSlashHandler::build_workflow_context_append` unit test.
    #[test]
    fn context_append_content_is_preserved() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_workflow_file(tmp.path(), "Test WF");
        let launch = EngineWorkflowLauncher
            .start("Test WF", Some(tmp.path()), None)
            .expect("load succeeds");
        let ctx = launch.context_append;
        assert!(ctx.starts_with("--- WORKFLOW ---"));
        assert!(ctx.ends_with("--- WORKFLOW END ---"));
        assert!(ctx.contains("Test Workflow"));
        assert!(ctx.contains("A test workflow"));
        assert!(ctx.contains("workflow_verify"));
        assert!(ctx.contains("workflow_jump"));
    }

    /// Equivalent coverage of the former private
    /// `WorkflowSlashHandler::build_goal_message` unit test (the
    /// workflow-side builder renders the identical
    /// `[workflow goal] Step {id}: {name}\n\n{goal}` format).
    #[test]
    fn goal_message_content_is_preserved() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_workflow_file(tmp.path(), "Test WF");
        let launch = EngineWorkflowLauncher
            .start("Test WF", Some(tmp.path()), None)
            .expect("load succeeds");
        let goal = launch.goal_message;
        assert!(goal.contains("[workflow goal]"));
        assert!(goal.contains("Step 0"));
        assert!(goal.contains("Step Zero"));
        assert!(goal.contains("Do the first thing"));
    }

    // ── Launch success（构造 + 加载成功） ──────────────────────────────

    #[test]
    fn load_success_returns_launch_data_with_downcastable_run() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_workflow_file(tmp.path(), "Test WF");

        let launch = EngineWorkflowLauncher
            .start("Test WF", Some(tmp.path()), None)
            .expect("Level 1 hit");

        assert_eq!(launch.first_step_name, "Step Zero");
        assert!(launch.context_append.contains("--- WORKFLOW ---"));
        assert!(launch.goal_message.contains("[workflow goal]"));
        // The run handle must downcast to the serialized run (checkpoint)
        // form — the contract `SlashSessionQuery` implementations rely on.
        let run = launch
            .run
            .downcast::<serde_json::Value>()
            .expect("run handle downcasts to the serialized run value");
        let run: WorkflowRun = serde_json::from_value(*run).expect("value decodes as a run");
        assert_eq!(run.current_step, 0);
        assert_eq!(run.definition_name, "Test Workflow");
    }

    #[test]
    fn level2_global_fallback_works_through_adapter() {
        let global = tempfile::TempDir::new().unwrap();
        write_workflow_file(global.path(), "Test WF");
        let empty_workspace = tempfile::TempDir::new().unwrap();

        let launch = EngineWorkflowLauncher
            .start("Test WF", Some(empty_workspace.path()), Some(global.path()))
            .expect("Level 2 hit");
        assert_eq!(launch.first_step_name, "Step Zero");
    }

    // ── Load failure（加载失败：错误路径 + Display 逐字保持） ─────────

    #[test]
    fn not_found_error_preserves_display() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = EngineWorkflowLauncher
            .start("NonExistent", Some(tmp.path()), None)
            .expect_err("all levels miss");
        match &err {
            WorkflowLoadError::DefinitionNotFound(name) => assert_eq!(name, "NonExistent"),
            other => panic!("expected DefinitionNotFound, got {other:?}"),
        }
        assert_eq!(
            err.to_string(),
            "workflow definition not found: NonExistent"
        );
    }

    #[test]
    fn parse_error_preserves_display() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wf_dir = tmp.path().join("workflows").join("Broken WF");
        std::fs::create_dir_all(&wf_dir).unwrap();
        // Frontmatter present but YAML malformed → parse failure.
        std::fs::write(
            wf_dir.join("SKILL.md"),
            "---\nid: [unclosed\n---\n\nBody.\n",
        )
        .unwrap();

        let err = EngineWorkflowLauncher
            .start("Broken WF", Some(tmp.path()), None)
            .expect_err("parse must fail");
        assert!(matches!(err, WorkflowLoadError::ParseError(_)));
        assert!(
            err.to_string()
                .starts_with("failed to parse workflow definition: "),
            "Display must match the workflow-side original: {err}"
        );
    }

    #[test]
    fn invalid_definition_error_preserves_display() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wf_dir = tmp.path().join("workflows").join("NoFrontmatter");
        std::fs::create_dir_all(&wf_dir).unwrap();
        // No frontmatter → invalid definition.
        std::fs::write(wf_dir.join("SKILL.md"), "plain text, no yaml frontmatter\n").unwrap();

        let err = EngineWorkflowLauncher
            .start("NoFrontmatter", Some(tmp.path()), None)
            .expect_err("frontmatter validation must fail");
        assert!(matches!(err, WorkflowLoadError::InvalidDefinition(_)));
        assert!(
            err.to_string().starts_with("invalid workflow definition: "),
            "Display must match the workflow-side original: {err}"
        );
    }

    #[test]
    fn error_mapping_covers_all_variants_with_display_intact() {
        let cases: Vec<(WorkflowError, WorkflowLoadError)> = vec![
            (
                WorkflowError::InvalidDefinition("bad".into()),
                WorkflowLoadError::InvalidDefinition("bad".into()),
            ),
            (
                WorkflowError::ParseError("oops".into()),
                WorkflowLoadError::ParseError("oops".into()),
            ),
            (
                WorkflowError::DefinitionNotFound("wf".into()),
                WorkflowLoadError::DefinitionNotFound("wf".into()),
            ),
            (
                WorkflowError::StepNotFound(3),
                WorkflowLoadError::Other("step not found: 3".into()),
            ),
        ];
        for (workflow_err, slash_err) in cases {
            assert_eq!(workflow_err.to_string(), slash_err.to_string());
        }
    }

    // ── Factory ─────────────────────────────────────────────────────────

    #[test]
    fn factory_returns_trait_object() {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher = engine_workflow_launcher();
        assert!(launcher.start("anything", Some(tmp.path()), None).is_err());
    }
}
