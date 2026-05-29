//! Permission Engine - Workspace path authorization.

use std::path::Path;

/// Check if a path belongs to the agent-user's workspace.
///
/// Uses syntax-level normalization (handles `..` and `.`) without
/// touching the filesystem. Performs boundary-aware prefix matching
/// to prevent `test-user` from matching `test-user2`.
pub(super) fn is_workspace_path(
    data_root: &Path,
    agent_id: &str,
    user_id: &str,
    path: &str,
) -> bool {
    let prefix = data_root.join("workspaces").join(agent_id).join(user_id);
    let norm_prefix = normalize_path(prefix.to_str().unwrap_or(""));
    let norm_path = normalize_path(path);
    norm_path == norm_prefix || norm_path.starts_with(&format!("{}/", norm_prefix))
}

/// Normalize a path by resolving `.` and `..` components syntactically.
pub(super) fn normalize_path(path: &str) -> String {
    let mut components: Vec<&str> = Vec::new();
    for component in Path::new(path).components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            std::path::Component::Normal(c) => {
                components.push(c.to_str().unwrap_or(""));
            }
            _ => {}
        }
    }
    let mut normalized = String::new();
    for comp in &components {
        normalized.push('/');
        normalized.push_str(comp);
    }
    if normalized.is_empty() {
        "/".to_string()
    } else {
        normalized
    }
}
