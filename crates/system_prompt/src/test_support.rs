//! Shared test-only helpers for agent workspace construction.

/// Create the agent workspace directory `agents/<agent_id>` under `base`,
/// returning the created directory path.
pub(crate) fn create_agent_workspace(base: &std::path::Path, agent_id: &str) -> std::path::PathBuf {
    let ws = base.join("agents").join(agent_id);
    std::fs::create_dir_all(&ws).unwrap();
    ws
}

/// Create the agent workspace directory under `base` and write `content`
/// into a single file `file_name` (e.g. `BOOTSTRAP.md`) inside it,
/// returning the workspace path.
pub(crate) fn create_agent_workspace_with_file(
    base: &std::path::Path,
    agent_id: &str,
    file_name: &str,
    content: &str,
) -> std::path::PathBuf {
    let ws = create_agent_workspace(base, agent_id);
    std::fs::write(ws.join(file_name), content).unwrap();
    ws
}
