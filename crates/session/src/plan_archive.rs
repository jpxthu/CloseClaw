//! Plan file automatic archival.
//!
//! Scans the `plans/` directory for completed plan files and archives
//! those whose last modification time exceeds a configurable threshold.
//! Archived files are moved to `plans/archive/` via `std::fs::rename`.

use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Default archival threshold in days.
pub const DEFAULT_THRESHOLD_DAYS: u64 = 7;

/// Configuration for plan archival behavior.
#[derive(Debug, Clone)]
pub struct ArchiveConfig {
    /// Number of days after last modification before archival.
    pub threshold_days: u64,
}

impl Default for ArchiveConfig {
    fn default() -> Self {
        Self {
            threshold_days: DEFAULT_THRESHOLD_DAYS,
        }
    }
}

/// Handles automatic archival of completed plan files.
///
/// Scans `plans/` for `.md` files where all step markers are in
/// terminal state (`[x]` done or `[~]` skipped), and moves those
/// older than the configured threshold to `plans/archive/`.
#[derive(Debug)]
pub struct PlanArchiver {
    config: ArchiveConfig,
}

impl PlanArchiver {
    /// Create a new archiver with the given threshold.
    pub fn new(threshold_days: u64) -> Self {
        Self {
            config: ArchiveConfig { threshold_days },
        }
    }

    /// Create a new archiver using default settings (7-day threshold).
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_THRESHOLD_DAYS)
    }

    /// Scan and archive completed plans in the given workspace.
    ///
    /// Returns the number of files archived.
    pub fn archive(&self, workdir: &Path) -> Result<u64, ArchiveError> {
        let plans_dir = workdir.join("plans");

        if !plans_dir.is_dir() {
            debug!("plans/ directory does not exist, skipping archival");
            return Ok(0);
        }

        let archive_dir = plans_dir.join("archive");
        std::fs::create_dir_all(&archive_dir)?;

        let now = chrono::Utc::now();
        let threshold = chrono::Duration::days(self.config.threshold_days as i64);
        let mut archived_count = 0u64;

        let entries = std::fs::read_dir(&plans_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Only process .md files
            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }

            // Skip files inside archive/ directory
            if path.starts_with(&archive_dir) {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    warn!("failed to read {}: {e}", path.display());
                    continue;
                }
            };

            if !is_completed_plan(&content) {
                continue;
            }

            let metadata = std::fs::metadata(&path)?;
            let mtime = metadata
                .modified()
                .ok()
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt
                })
                .unwrap_or_else(chrono::Utc::now);

            if now.signed_duration_since(mtime) > threshold {
                let file_name = path
                    .file_name()
                    .ok_or_else(|| ArchiveError::InvalidPath(path.clone()))?;
                let dest = archive_dir.join(file_name);

                info!("archiving {} → {}", path.display(), dest.display());
                std::fs::rename(&path, &dest)?;
                archived_count += 1;
            } else {
                debug!("skipping {} (not old enough)", path.display());
            }
        }

        Ok(archived_count)
    }
}

/// Archive completed plans in a workspace.
///
/// Convenience function using default settings (7-day threshold).
/// Returns the number of files archived.
pub fn archive_completed_plans(workdir: &Path) -> Result<u64, ArchiveError> {
    PlanArchiver::with_defaults().archive(workdir)
}

/// Archive completed plans with a custom threshold.
///
/// Returns the number of files archived.
pub fn archive_completed_plans_with_threshold(
    workdir: &Path,
    threshold_days: u64,
) -> Result<u64, ArchiveError> {
    PlanArchiver::new(threshold_days).archive(workdir)
}

/// Step marker state extracted from a plan's Tasks section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepState {
    /// `[x]` — completed
    Done,
    /// `[ ]` — not started
    Pending,
    /// `[-]` — in progress
    InProgress,
    /// `[!]` — failed
    Failed,
    /// `[~]` — skipped
    Skipped,
}

/// Extract all step markers from a plan file's Tasks section.
///
/// Looks for lines starting with `- [` or `* [` (with optional
/// leading whitespace) anywhere in the file. Each marker is
/// classified into one of [`StepState`] variants.
///
/// Returns an empty vec if no step markers are found.
pub(crate) fn parse_step_markers(content: &str) -> Vec<StepState> {
    let mut states = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- [") || trimmed.starts_with("* [") {
            let bracket_start = trimmed.find('[').unwrap() + 1;
            let bracket_end = trimmed[bracket_start..].find(']');
            if let Some(end) = bracket_end {
                let marker = &trimmed[bracket_start..bracket_start + end];
                let state = match marker {
                    "x" => StepState::Done,
                    " " => StepState::Pending,
                    "-" => StepState::InProgress,
                    "!" => StepState::Failed,
                    "~" => StepState::Skipped,
                    _ => continue,
                };
                states.push(state);
            }
        }
    }

    states
}

/// Check if a plan is considered completed.
///
/// A plan is completed when the Tasks section contains at least one
/// step marker and **all** steps are in a terminal state:
/// - `[x]` (done) or `[~]` (skipped) — considered completed.
/// - `[ ]`, `[-]`, `[!]` — plan is still active or failed, not
///   eligible for archival.
///
/// An empty Tasks section (no step markers) is **not** treated as
/// completed to avoid archiving empty/plans that were never started.
pub(crate) fn is_completed_plan(content: &str) -> bool {
    let states = parse_step_markers(content);
    if states.is_empty() {
        return false;
    }
    states
        .iter()
        .all(|s| matches!(s, StepState::Done | StepState::Skipped))
}

/// Errors that can occur during plan archival.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    /// The file path could not be resolved.
    #[error("invalid path: {0}")]
    InvalidPath(PathBuf),

    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
