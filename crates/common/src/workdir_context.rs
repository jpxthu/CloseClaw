//! WorkdirContext — working directory context for tools.
//!
//! Moved here to decouple the tools crate from the main crate's
//! `system_prompt::workdir` module.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Workdir context returned by build_workdir_context
#[derive(Debug, Clone)]
pub struct WorkdirContext {
    /// Absolute path to the working directory
    pub path: String,
    /// Whether this is a git repository
    pub has_git: bool,
    /// Current git branch (if has_git)
    pub branch: Option<String>,
    /// Number of uncommitted changes (if has_git)
    pub recent_changes: usize,
}

/// Build a WorkdirContext for the given path.
///
/// Resolves relative paths against `cwd`. Canonicalizes the result.
pub fn build_workdir_context(path: &str) -> WorkdirContext {
    let abs_path = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };

    let canonical = abs_path.canonicalize().unwrap_or(abs_path);
    let has_git = is_git_repo(&canonical);
    let branch = if has_git {
        get_git_branch(&canonical)
    } else {
        None
    };
    let recent_changes = if has_git {
        count_uncommitted_changes(&canonical)
    } else {
        0
    };

    WorkdirContext {
        path: canonical.to_string_lossy().to_string(),
        has_git,
        branch,
        recent_changes,
    }
}

/// Build a git status string for the given path (None if not a git repo).
pub fn build_git_status_for(path: &str) -> Option<String> {
    let p = Path::new(path);

    if !is_git_repo(p) {
        return None;
    }

    let branch = get_git_branch(p)?;
    let changes = count_uncommitted_changes(p);

    let status_summary = if changes == 0 {
        "clean".to_string()
    } else {
        format!("{} uncommitted change(s)", changes)
    };

    Some(format!(
        "On branch {}\n  status: {}",
        branch, status_summary
    ))
}

fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists() || find_git_root(path).is_some()
}

fn find_git_root(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path.to_path_buf());
    while let Some(p) = current {
        if p.join(".git").exists() {
            return Some(p);
        }
        current = p.parent().map(|p| p.to_path_buf());
    }
    None
}

fn get_git_branch(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn count_uncommitted_changes(path: &Path) -> usize {
    let staged = git_status_count(path, "--cached");
    let unstaged = git_status_count(path, "");
    let untracked = git_status_count(path, "--others");

    staged + unstaged + untracked
}

fn git_status_count(path: &Path, extra_arg: &str) -> usize {
    let args: Vec<&str> = if extra_arg.is_empty() {
        vec!["status", "--porcelain"]
    } else {
        vec!["status", "--porcelain", extra_arg]
    };

    let output = Command::new("git")
        .args(&args)
        .current_dir(path)
        .output()
        .ok();

    output
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.is_empty())
                .count()
        })
        .unwrap_or(0)
}
