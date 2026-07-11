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
    if norm_path == norm_prefix || norm_path.starts_with(&format!("{}/", norm_prefix)) {
        return true;
    }
    is_nested_workspace_path(data_root, agent_id, user_id, &norm_path)
}

/// Check if a normalized path is a nested workspace path for the given agent-user.
///
/// A nested workspace path satisfies:
/// - Starts with `{data_root}/workspaces/`
/// - Ends with `/{agent_id}/{user_id}` or `/{agent_id}/{user_id}/...`
fn is_nested_workspace_path(
    data_root: &Path,
    agent_id: &str,
    user_id: &str,
    norm_path: &str,
) -> bool {
    let ws_prefix = normalize_path(&data_root.join("workspaces").to_string_lossy().into_owned());
    if norm_path != ws_prefix && !norm_path.starts_with(&format!("{}/", ws_prefix)) {
        return false;
    }
    // The path is under data_root/workspaces/. Check if the last two components
    // match agent_id/user_id, using boundary-aware matching.
    let suffix = format!("/{}/{}", agent_id, user_id);
    let suffix_with_slash = format!("{}/", suffix);
    norm_path.ends_with(&suffix) || norm_path.contains(&suffix_with_slash)
}

/// Check if a path is inside the config directory but not in any workspace.
///
/// Config directory paths (e.g. `data_root/agents/...`, `data_root/permissions.json`)
/// are hard-coded to be denied. This prevents agents from reading or writing
/// permission configuration files regardless of rules or defaults.
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

/// Public API: check if a file path targets a config file.
///
/// A config file is any path inside `data_root` that is NOT under
/// `data_root/workspaces/`.  This is used by the tool layer to decide
/// whether a file-write operation should also trigger the ConfigWrite
/// permission dimension.
///
/// # Arguments
///
/// * `data_root` – the permission data root directory.
/// * `path` – absolute file path to check.
pub fn is_config_file_path(data_root: &Path, path: &str) -> bool {
    is_config_dir_path(data_root, path)
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
