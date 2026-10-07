//! gitStatus helper re-export
//!
//! Re-exports `build_git_status_for` from `closeclaw_common` for
//! use within the system_prompt crate.

pub(crate) use closeclaw_common::tool_trait::build_git_status_for;

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::tool_trait::build_workdir_context;

    #[test]
    fn test_build_workdir_context_with_temp_dir() {
        let temp = std::env::temp_dir();
        let ctx = build_workdir_context(&temp.to_string_lossy());
        // The context path must be the canonicalized absolute form of the
        // input directory, regardless of where the temp dir actually lives.
        assert_eq!(ctx.path, temp.canonicalize().unwrap().to_string_lossy());
        assert!(std::path::Path::new(&ctx.path).is_absolute());
    }

    #[test]
    fn test_build_workdir_context_git_detection() {
        let ctx = build_workdir_context(env!("CARGO_MANIFEST_DIR"));
        assert!(ctx.has_git, "expected git repo at {}", ctx.path);
        assert!(!ctx.path.is_empty());
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
        let dir = std::env::temp_dir();
        // Do not assume the temp dir is never a repo: decide repo-ness with
        // the same detection the production path uses, and only assert the
        // non-repo contract when it actually applies.
        if build_workdir_context(&dir.to_string_lossy()).has_git {
            return;
        }
        assert!(build_git_status_for(&dir.to_string_lossy()).is_none());
    }
}
