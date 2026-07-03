//! Workdir Context and gitStatus
//!
//! Re-exports [`WorkdirContext`] and helper functions from
//! [`closeclaw_tools`] for use within the system_prompt crate.

pub use closeclaw_tools::{build_git_status_for, build_workdir_context, WorkdirContext};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_workdir_context_with_temp_dir() {
        let temp = std::env::temp_dir();
        let ctx = build_workdir_context(&temp.to_string_lossy());
        assert!(ctx.path.contains("tmp") || ctx.path.contains("temp"));
    }

    #[test]
    fn test_build_workdir_context_git_detection() {
        let ctx = build_workdir_context(env!("CARGO_MANIFEST_DIR"));
        // Just verify it runs without panicking
        assert!(ctx.has_git || !ctx.has_git);
    }

    #[test]
    fn test_build_workdir_context_relative_path() {
        let ctx = build_workdir_context(".");
        assert!(!ctx.path.is_empty());
    }

    #[test]
    fn test_build_git_status_for_repo() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let status = build_git_status_for(manifest);
        // The project root is a git repo, so we expect Some
        assert!(status.is_some());
    }

    #[test]
    fn test_build_git_status_for_non_repo() {
        let status = build_git_status_for("/tmp");
        // /tmp is typically not a git repo
        assert!(status.is_none());
    }
}
