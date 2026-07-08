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

/// Check if a path is inside the config directory but not in any workspace.
///
/// Config directory paths (e.g. `data_root/agents/...`, `data_root/permissions.json`)
/// are hard-coded to be denied. This prevents agents from reading or writing
/// permission configuration files regardless of rules or defaults.
#[allow(dead_code, reason = "used in Step 1.2 integrate into evaluate()")]
pub(super) fn is_config_dir_path(data_root: &Path, path: &str) -> bool {
    let norm_root = normalize_path(data_root.to_str().unwrap_or(""));
    let norm_path = normalize_path(path);

    // Path must be inside data_root
    if norm_path != norm_root && !norm_path.starts_with(&format!("{}/", norm_root)) {
        return false;
    }

    // Paths inside data_root/workspaces/ are allowed (handled by is_workspace_path)
    let ws_prefix = format!("{}/workspaces/", norm_root);
    norm_path == norm_root || !norm_path.starts_with(&ws_prefix)
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
