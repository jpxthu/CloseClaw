//! Workspace directory management for agent-user session isolation.
//!
//! Each agent-user combination gets a dedicated workspace directory under
//! `{config_dir}/workspaces/{agent_id}/{user_id}/`.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// Errors that can occur during workspace operations.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// Agent ID contains invalid characters (path traversal or separators).
    #[error("Agent ID '{0}' contains invalid characters")]
    InvalidAgentId(String),

    /// User ID contains invalid characters (path traversal or separators).
    #[error("User ID '{0}' contains invalid characters")]
    InvalidUserId(String),

    /// Failed to create workspace directory.
    #[error("Failed to create workspace directory: {0}")]
    CreationFailed(#[from] std::io::Error),
}

/// Validates that a single path component (agent_id or user_id) is safe.
///
/// # Errors
///
/// Returns `WorkspaceError` if the component:
/// - Is empty
/// - Contains `..` (path traversal)
/// - Contains `/` or `\` (directory separators)
/// - Is `.` or `..` (special directory names)
///
/// # Examples
///
/// ```
/// # use closeclaw::session::workspace::validate_path_component;
/// assert!(validate_path_component("user123").is_ok());
/// assert!(validate_path_component("").is_err());
/// assert!(validate_path_component("../etc").is_err());
/// ```
pub fn validate_path_component(id: &str) -> Result<(), WorkspaceError> {
    if id.is_empty() {
        return Err(WorkspaceError::InvalidAgentId(String::new()));
    }

    if id == "." || id == ".." {
        return Err(WorkspaceError::InvalidAgentId(id.to_string()));
    }

    if id.contains("..") {
        return Err(WorkspaceError::InvalidAgentId(id.to_string()));
    }

    if id.contains('/') || id.contains('\\') {
        return Err(WorkspaceError::InvalidAgentId(id.to_string()));
    }

    if id.contains('\0') {
        return Err(WorkspaceError::InvalidAgentId(id.to_string()));
    }

    Ok(())
}

/// Ensures the workspace directory exists for a given agent-user pair.
///
/// The workspace path is computed as:
/// `{config_dir}/workspaces/{agent_id}/{user_id}/`
///
/// # Errors
///
/// Returns `WorkspaceError` if:
/// - `agent_id` or `user_id` fails validation
/// - Directory creation fails
///
/// # Examples
///
/// ```
/// # use std::path::Path;
/// # use closeclaw::session::workspace::ensure_workspace_dir;
/// # let temp = tempfile::tempdir().unwrap();
/// # let config_dir = temp.path();
/// let workspace = ensure_workspace_dir(config_dir, "agent1", "user1").unwrap();
/// assert!(workspace.exists());
/// ```
pub fn ensure_workspace_dir(
    config_dir: &Path,
    agent_id: &str,
    user_id: &str,
) -> Result<PathBuf, WorkspaceError> {
    validate_path_component(agent_id).map_err(|e| match e {
        WorkspaceError::InvalidAgentId(_) => WorkspaceError::InvalidAgentId(agent_id.to_string()),
        _ => e,
    })?;

    validate_path_component(user_id).map_err(|e| match e {
        WorkspaceError::InvalidAgentId(_) => WorkspaceError::InvalidUserId(user_id.to_string()),
        _ => e,
    })?;

    let workspace_path = config_dir.join("workspaces").join(agent_id).join(user_id);

    std::fs::create_dir_all(&workspace_path)?;

    Ok(workspace_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_path_component_ok() {
        assert!(validate_path_component("user123").is_ok());
        assert!(validate_path_component("agent_abc").is_ok());
        assert!(validate_path_component("a").is_ok());
    }

    #[test]
    fn test_validate_path_component_empty() {
        let result = validate_path_component("");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_path_component_dot() {
        assert!(validate_path_component(".").is_err());
        assert!(validate_path_component("..").is_err());
    }

    #[test]
    fn test_validate_path_component_traversal() {
        assert!(validate_path_component("../etc").is_err());
        assert!(validate_path_component("foo../bar").is_err());
    }

    #[test]
    fn test_validate_path_component_slash() {
        assert!(validate_path_component("foo/bar").is_err());
        assert!(validate_path_component("foo/bar/baz").is_err());
    }

    #[test]
    fn test_validate_path_component_backslash() {
        assert!(validate_path_component("foo\\bar").is_err());
    }

    #[test]
    fn test_validate_path_component_null() {
        assert!(validate_path_component("foo\0bar").is_err());
    }

    #[test]
    fn test_ensure_workspace_dir_creates() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        let workspace = ensure_workspace_dir(config_dir, "agent1", "user1").unwrap();
        assert!(workspace.exists());
        assert!(workspace.is_dir());
    }

    #[test]
    fn test_ensure_workspace_dir_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        let workspace1 = ensure_workspace_dir(config_dir, "agent1", "user1").unwrap();
        let workspace2 = ensure_workspace_dir(config_dir, "agent1", "user1").unwrap();
        assert_eq!(workspace1, workspace2);
    }

    #[test]
    fn test_ensure_workspace_dir_traversal_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        assert!(ensure_workspace_dir(config_dir, "../etc", "user1").is_err());
        assert!(ensure_workspace_dir(config_dir, "agent1", "..\\foo").is_err());
    }
}
