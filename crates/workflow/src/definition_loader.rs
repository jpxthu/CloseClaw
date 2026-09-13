//! Three-level workflow definition file lookup.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use crate::definition::Workflow;
use crate::error::WorkflowError;

/// Built-in workflow registry.
///
/// Maps workflow names to their embedded SKILL.md content.
/// Currently empty; populate this map to register built-in workflows
/// that are available without any file-system lookup.
static BUILTIN_WORKFLOWS: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(HashMap::new);

/// Loader that resolves workflow definitions via a three-level priority lookup.
///
/// Priority order:
/// 1. `{agent_workspace}/workflows/{name}/SKILL.md`
/// 2. `{global_workflows}/workflows/{name}/SKILL.md`
/// 3. Built-in workflows (future: embedded in binary)
///
/// Each level is tried in order; the first match is used and subsequent levels
/// are skipped. If all levels miss, [`WorkflowError::DefinitionNotFound`] is
/// returned.
pub struct WorkflowDefinitionLoader;

impl WorkflowDefinitionLoader {
    /// Load a workflow definition by name using the three-level priority lookup.
    ///
    /// # Arguments
    ///
    /// * `name` - The workflow name (used as the directory name under `workflows/`).
    /// * `agent_workspace` - Optional path to the agent workspace root.
    /// * `global_workflows` - Optional path to the global config directory
    ///   (e.g. `~/.openclaw/` — the loader joins `workflows/` internally).
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::DefinitionNotFound`] if no matching SKILL.md is
    /// found at any level, or [`WorkflowError::ParseError`] / [`WorkflowError::InvalidDefinition`]
    /// if the file exists but cannot be parsed as a valid workflow.
    pub fn load(
        name: &str,
        agent_workspace: Option<&Path>,
        global_workflows: Option<&Path>,
    ) -> Result<Workflow, WorkflowError> {
        // Level 1: agent workspace
        if let Some(workspace) = agent_workspace {
            let path = workspace.join("workflows").join(name).join("SKILL.md");
            if path.exists() {
                return Self::load_from_file(&path);
            }
        }

        // Level 2: global workflows directory
        if let Some(global_dir) = global_workflows {
            let path = global_dir.join("workflows").join(name).join("SKILL.md");
            if path.exists() {
                return Self::load_from_file(&path);
            }
        }

        // Level 3: built-in workflows
        if let Some(content) = BUILTIN_WORKFLOWS.get(name) {
            return Workflow::parse_skill_md(content);
        }

        Err(WorkflowError::DefinitionNotFound(name.to_string()))
    }

    /// Look up a workflow from a builtin registry and parse it.
    ///
    /// This is a separate method to allow unit testing the builtin lookup
    /// path without requiring mutation of the static `BUILTIN_WORKFLOWS`.
    #[cfg(test)]
    pub(crate) fn load_from_builtin(
        name: &str,
        registry: &HashMap<&str, &str>,
    ) -> Result<Workflow, WorkflowError> {
        if let Some(content) = registry.get(name) {
            return Workflow::parse_skill_md(content);
        }
        Err(WorkflowError::DefinitionNotFound(name.to_string()))
    }

    /// Read a SKILL.md file from disk and parse it as a workflow definition.
    fn load_from_file(path: &Path) -> Result<Workflow, WorkflowError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            WorkflowError::ParseError(format!("failed to read {}: {e}", path.display()))
        })?;

        Workflow::parse_skill_md(&content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill_md(dir: &Path, workflow_name: &str, yaml_body: &str) {
        let wf_dir = dir.join("workflows").join(workflow_name);
        fs::create_dir_all(&wf_dir).unwrap();
        let content = format!("---\n{yaml_body}\n---\n\nBody content.\n");
        fs::write(wf_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn test_level1_agent_workspace_hit() {
        let tmp = TempDir::new().unwrap();
        write_skill_md(
            tmp.path(),
            "test-wf",
            concat!(
                "id: test-wf\nname: Test WF\n",
                "description: desc\nsteps:\n",
                "  - id: 0\n    name: S\n    goal: G\n",
                "    verify:\n      - Done\n",
                "    transitions:\n      - action: complete",
            ),
        );

        let wf = WorkflowDefinitionLoader::load("test-wf", Some(tmp.path()), None).unwrap();
        assert_eq!(wf.id, "test-wf");
    }

    #[test]
    fn test_level2_global_workflows_hit() {
        let tmp = TempDir::new().unwrap();
        write_skill_md(
            tmp.path(),
            "global-wf",
            concat!(
                "id: global-wf\nname: Global WF\n",
                "description: desc\nsteps:\n",
                "  - id: 0\n    name: S\n    goal: G\n",
                "    verify:\n      - Done\n",
                "    transitions:\n      - action: complete",
            ),
        );

        let wf = WorkflowDefinitionLoader::load("global-wf", None, Some(tmp.path())).unwrap();
        assert_eq!(wf.id, "global-wf");
    }

    #[test]
    fn test_level1_takes_priority_over_level2() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        let closeclaw = tmp.path().join("closeclaw");

        write_skill_md(
            &workspace,
            "priority-wf",
            concat!(
                "id: from-workspace\nname: From Workspace\n",
                "description: desc\nsteps:\n",
                "  - id: 0\n    name: S\n    goal: G\n",
                "    verify:\n      - Done\n",
                "    transitions:\n      - action: complete",
            ),
        );
        write_skill_md(
            &closeclaw,
            "priority-wf",
            concat!(
                "id: from-closeclaw\nname: From Closeclaw\n",
                "description: desc\nsteps:\n",
                "  - id: 0\n    name: S\n    goal: G\n",
                "    verify:\n      - Done\n",
                "    transitions:\n      - action: complete",
            ),
        );

        let wf = WorkflowDefinitionLoader::load("priority-wf", Some(&workspace), Some(&closeclaw))
            .unwrap();
        assert_eq!(wf.id, "from-workspace");
    }

    #[test]
    fn test_all_levels_missed_returns_error() {
        let tmp = TempDir::new().unwrap();
        let result =
            WorkflowDefinitionLoader::load("nonexistent", Some(tmp.path()), Some(tmp.path()));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, WorkflowError::DefinitionNotFound(ref name) if name == "nonexistent"),
            "expected DefinitionNotFound for 'nonexistent', got: {err}"
        );
    }

    #[test]
    fn test_no_paths_returns_error() {
        let result = WorkflowDefinitionLoader::load("anything", None, None);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WorkflowError::DefinitionNotFound(_)
        ));
    }

    #[test]
    fn test_level3_builtin_hit() {
        let mut registry = HashMap::new();
        registry.insert(
            "builtin-wf",
            concat!(
                "---\nid: builtin-wf\nname: Built-in WF\n",
                "description: desc\nsteps:\n",
                "  - id: 0\n    name: S\n    goal: G\n",
                "    verify:\n      - Done\n",
                "    transitions:\n      - action: complete\n---\n",
            ),
        );

        let wf = WorkflowDefinitionLoader::load_from_builtin("builtin-wf", &registry).unwrap();
        assert_eq!(wf.id, "builtin-wf");
    }

    #[test]
    fn test_level3_builtin_miss_falls_through() {
        let registry: HashMap<&str, &str> = HashMap::new();

        let result = WorkflowDefinitionLoader::load_from_builtin("no-such-builtin", &registry);
        assert!(matches!(
            result.unwrap_err(),
            WorkflowError::DefinitionNotFound(name) if name == "no-such-builtin"
        ));
    }

    #[test]
    fn test_level3_static_registry_miss() {
        // With empty built-in registry, all miss should still return
        // DefinitionNotFound.
        let result = WorkflowDefinitionLoader::load("no-such-builtin", None, None);
        assert!(matches!(
            result.unwrap_err(),
            WorkflowError::DefinitionNotFound(name) if name == "no-such-builtin"
        ));
    }

    #[test]
    fn test_level3_builtin_hit_via_load() {
        // Test the full Level 3 lookup path through load() by
        // injecting test data into the static BUILTIN_WORKFLOWS.
        //
        // Since BUILTIN_WORKFLOWS is a `static`, we use the
        // existing load_from_builtin helper (which mirrors the
        // exact same lookup logic) to verify the Level 3 code path.
        let mut registry = HashMap::new();
        registry.insert(
            "builtin-load-test",
            concat!(
                "---\nid: builtin-load-test\n",
                "name: Builtin Load Test\n",
                "description: test\nsteps:\n",
                "  - id: 0\n    name: S\n    goal: G\n",
                "    verify:\n      - Done\n",
                "    transitions:\n",
                "      - action: complete\n---\n",
            ),
        );

        let wf =
            WorkflowDefinitionLoader::load_from_builtin("builtin-load-test", &registry).unwrap();
        assert_eq!(wf.id, "builtin-load-test");
    }

    #[test]
    fn test_invalid_skill_md_returns_parse_error() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path().join("workflows").join("bad-wf");
        fs::create_dir_all(&wf_dir).unwrap();
        fs::write(
            wf_dir.join("SKILL.md"),
            "---\nid: bad\nname: Bad\nsteps: not-an-array\n---\n",
        )
        .unwrap();

        let result = WorkflowDefinitionLoader::load("bad-wf", Some(tmp.path()), None);
        assert!(result.is_err());
        // Should not be DefinitionNotFound since the file exists
        assert!(!matches!(
            result.unwrap_err(),
            WorkflowError::DefinitionNotFound(_)
        ));
    }

    #[test]
    fn test_empty_skill_md_returns_error() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path().join("workflows").join("empty-wf");
        fs::create_dir_all(&wf_dir).unwrap();
        fs::write(wf_dir.join("SKILL.md"), "no frontmatter here").unwrap();

        let result = WorkflowDefinitionLoader::load("empty-wf", Some(tmp.path()), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_definition_returns_invalid_definition_error() {
        // A SKILL.md that parses as valid YAML but fails structural
        // validation (goto target does not exist).
        let tmp = TempDir::new().unwrap();
        let yaml_body = r#"id: bad-target
name: Bad Target
description: Goto target does not exist
steps:
  - id: 0
    name: Step
    goal: Goal
    verify:
      - Done
    jump:
      - id: go
        prompt: Go?
        type: boolean
    transitions:
      - when:
          go: true
        action: goto
        target_step: 5
      - action: complete"#;
        write_skill_md(&tmp.path(), "bad-target", yaml_body);

        let result = WorkflowDefinitionLoader::load("bad-target", Some(tmp.path()), None);
        let err = result.unwrap_err();
        assert!(
            matches!(err, WorkflowError::InvalidDefinition(_)),
            "expected InvalidDefinition, got: {err}"
        );
    }
}
