//! Three-level workflow definition file lookup.

use std::path::Path;

use crate::definition::Workflow;
use crate::error::WorkflowError;

/// Loader that resolves workflow definitions via a three-level priority lookup.
///
/// Priority order:
/// 1. `{agent_workspace}/workflows/{name}/SKILL.md`
/// 2. `{dot_closeclaw}/workflows/{name}/SKILL.md`
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
    /// * `dot_closeclaw` - Optional path to the `.closeclaw` directory.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::DefinitionNotFound`] if no matching SKILL.md is
    /// found at any level, or [`WorkflowError::ParseError`] / [`WorkflowError::InvalidDefinition`]
    /// if the file exists but cannot be parsed as a valid workflow.
    pub fn load(
        name: &str,
        agent_workspace: Option<&Path>,
        dot_closeclaw: Option<&Path>,
    ) -> Result<Workflow, WorkflowError> {
        // Level 1: agent workspace
        if let Some(workspace) = agent_workspace {
            let path = workspace.join("workflows").join(name).join("SKILL.md");
            if path.exists() {
                return Self::load_from_file(&path);
            }
        }

        // Level 2: .closeclaw directory
        if let Some(closeclaw_dir) = dot_closeclaw {
            let path = closeclaw_dir.join("workflows").join(name).join("SKILL.md");
            if path.exists() {
                return Self::load_from_file(&path);
            }
        }

        // Level 3: built-in (placeholder for future embedded workflows)
        // Currently no built-in workflows are registered.
        // Future: check a known embedded registry here.

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
            "id: test-wf\nname: Test WF\ndescription: desc\nsteps:\n  - id: 0\n    name: S\n    goal: G",
        );

        let wf = WorkflowDefinitionLoader::load("test-wf", Some(tmp.path()), None).unwrap();
        assert_eq!(wf.id, "test-wf");
    }

    #[test]
    fn test_level2_dot_closeclaw_hit() {
        let tmp = TempDir::new().unwrap();
        write_skill_md(
            tmp.path(),
            "dot-wf",
            "id: dot-wf\nname: Dot WF\ndescription: desc\nsteps:\n  - id: 0\n    name: S\n    goal: G",
        );

        let wf = WorkflowDefinitionLoader::load("dot-wf", None, Some(tmp.path())).unwrap();
        assert_eq!(wf.id, "dot-wf");
    }

    #[test]
    fn test_level1_takes_priority_over_level2() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        let closeclaw = tmp.path().join("closeclaw");

        write_skill_md(
            &workspace,
            "priority-wf",
            "id: from-workspace\nname: From Workspace\ndescription: desc\nsteps:\n  - id: 0\n    name: S\n    goal: G",
        );
        write_skill_md(
            &closeclaw,
            "priority-wf",
            "id: from-closeclaw\nname: From Closeclaw\ndescription: desc\nsteps:\n  - id: 0\n    name: S\n    goal: G",
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
}
