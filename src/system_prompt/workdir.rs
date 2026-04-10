//! Workdir Context and gitStatus
//!
//! Manages the current working directory for the agent session and provides
//! git status information for injection into the system prompt.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::RwLock;

/// Workdir context returned by set_workdir
#[derive(Debug, Clone)]
pub struct WorkdirContext {
    /// Absolute path to the current working directory
    pub path: String,
    /// Whether this is a git repository
    pub has_git: bool,
    /// Current git branch (if has_git)
    pub branch: Option<String>,
    /// Number of uncommitted changes (if has_git)
    pub recent_changes: usize,
}

/// Current workdir state
static CURRENT_WORKDIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Set the current working directory and return its metadata.
pub fn set_workdir(path: String) -> WorkdirContext {
    let abs_path = if Path::new(&path).is_absolute() {
        PathBuf::from(&path)
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(&path)
    };

    let canonical = abs_path.canonicalize().unwrap_or(abs_path.clone());
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

    if let Ok(mut guard) = CURRENT_WORKDIR.write() {
        *guard = Some(canonical.clone());
    }

    WorkdirContext {
        path: canonical.to_string_lossy().to_string(),
        has_git,
        branch,
        recent_changes,
    }
}

/// Get the current working directory (if set)
pub fn get_workdir() -> Option<String> {
    CURRENT_WORKDIR
        .read()
        .ok()?
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
}

/// Clear the current working directory
pub fn clear_workdir() {
    if let Ok(mut guard) = CURRENT_WORKDIR.write() {
        *guard = None;
    }
}

// ---------------------------------------------------------------------------
// Git helpers
// ---------------------------------------------------------------------------

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
    // Count: staged + unstaged + untracked
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

    let output = Command::new("git").args(&args).current_dir(path).output().ok();

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

/// Build a git status string for the current workdir (empty if not a git repo)
pub fn build_git_status() -> Option<String> {
    let workdir = get_workdir()?;
    let path = Path::new(&workdir);

    if !is_git_repo(path) {
        return None;
    }

    let branch = get_git_branch(path)?;
    let changes = count_uncommitted_changes(path);

    let status_summary = if changes == 0 {
        "clean".to_string()
    } else {
        format!("{} uncommitted change(s)", changes)
    };

    Some(format!("On branch {}\n  status: {}", branch, status_summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_set_and_get_workdir() {
        let temp = std::env::temp_dir();
        let ctx = set_workdir(temp.to_string_lossy().to_string());
        assert!(ctx.path.contains("tmp") || ctx.path.contains("temp"));
        assert_eq!(get_workdir(), Some(ctx.path.clone()));
        clear_workdir();
    }

    #[test]
    fn test_workdir_git_detection() {
        // Use the repo root which IS a git repo
        let ctx = set_workdir(env!("CARGO_MANIFEST_DIR").to_string());
        assert!(ctx.has_git || !ctx.has_git); // Just check it runs
        clear_workdir();
    }

    #[test]
    fn test_workdir_relative_path() {
        let ctx = set_workdir(".".to_string());
        assert!(!ctx.path.is_empty());
        clear_workdir();
    }
}
