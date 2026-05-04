//! Skill Hot Reload - file system watcher for skill directory changes
//!
//! Watches skill directories and triggers a callback when files are added,
//! modified, or removed. Uses 300ms debounce to coalesce rapid changes.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use thiserror::Error;
use tracing::{debug, warn};

/// Errors that can occur when starting the skill watcher.
#[derive(Error, Debug)]
pub enum HotReloadError {
    #[error("failed to create watcher: {0}")]
    WatcherCreate(#[from] notify::Error),

    #[error("failed to watch path {path}: {source}")]
    WatchPath {
        path: PathBuf,
        source: notify::Error,
    },

    #[error("no skill directories provided")]
    NoDirectories,
}

/// Handle to the skill watcher. Dropping it stops the watcher.
///
/// # Examples
///
/// ```no_run
/// use closeclaw::skills::disk::{start_skill_watcher, SkillWatcherHandle};
/// use std::path::PathBuf;
///
/// let handle: SkillWatcherHandle = start_skill_watcher(
///     vec![PathBuf::from("/path/to/skills")],
///     Box::new(|| println!("skills changed")),
/// )
/// .unwrap();
/// // Watcher is active while `handle` is alive.
/// drop(handle); // watcher stops
/// ```
#[derive(Debug)]
pub struct SkillWatcherHandle {
    // Watcher is kept alive to continue watching; RAII drop stops it.
    _watcher: RecommendedWatcher,
}

/// Start watching skill directories for file changes.
///
/// Returns `HotReloadError::NoDirectories` if `skill_dirs` is empty.
/// Directories that do not exist are logged with a warning and skipped.
/// File change events are debounced by 300ms before invoking `on_change`.
///
/// # Examples
///
/// ```no_run
/// use closeclaw::skills::disk::start_skill_watcher;
/// use std::path::PathBuf;
///
/// let handle = start_skill_watcher(
///     vec![PathBuf::from("/path/to/skills")],
///     Box::new(|| println!("skill files changed")),
/// )
/// .unwrap();
/// // handle drop stops the watcher
/// ```
pub fn start_skill_watcher(
    skill_dirs: Vec<PathBuf>,
    on_change: Box<dyn Fn() + Send + 'static>,
) -> Result<SkillWatcherHandle, HotReloadError> {
    if skill_dirs.is_empty() {
        return Err(HotReloadError::NoDirectories);
    }

    // Filter to only directories that exist, warn on missing ones
    let dirs_to_watch: Vec<PathBuf> = skill_dirs
        .into_iter()
        .filter(|p| {
            if !p.exists() {
                warn!(
                    "skill watch directory does not exist, skipping: {}",
                    p.display()
                );
                false
            } else {
                true
            }
        })
        .collect();

    if dirs_to_watch.is_empty() {
        warn!("no valid skill directories to watch");
        // Return a no-op handle rather than error, to allow graceful startup
        return Ok(SkillWatcherHandle {
            _watcher: RecommendedWatcher::new(
                |_res: Result<Event, notify::Error>| {},
                NotifyConfig::default(),
            )?,
        });
    }

    let (tx, rx) = mpsc::channel();

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
        NotifyConfig::default(),
    )?;

    for dir in &dirs_to_watch {
        watcher
            .watch(dir, RecursiveMode::Recursive)
            .map_err(|source| HotReloadError::WatchPath {
                path: dir.clone(),
                source,
            })?;
    }

    std::thread::spawn(move || {
        run_watch_loop(rx, on_change);
    });

    Ok(SkillWatcherHandle { _watcher: watcher })
}

/// Run the watch loop with 300ms debounce on the receiver.
fn run_watch_loop(rx: mpsc::Receiver<Event>, on_change: Box<dyn Fn() + Send + 'static>) {
    let debounce = Duration::from_millis(300);
    let mut last_event_time: Option<Instant> = None;

    for event in rx {
        // Only act on create/modify/remove events
        if !matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) {
            continue;
        }

        let now = Instant::now();
        if let Some(last) = last_event_time {
            if now.duration_since(last) < debounce {
                debug!("debouncing skill directory change event");
                continue;
            }
        }
        last_event_time = Some(now);

        for path in &event.paths {
            debug!("skill directory changed: {}", path.display());
        }
        on_change();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn test_no_directories_error() {
        let called = Arc::new(Mutex::new(false));
        let called_clone = Arc::clone(&called);
        let cb = Box::new(move || {
            *called_clone.lock().unwrap() = true;
        });
        let result = start_skill_watcher(vec![], cb);
        assert!(matches!(result, Err(HotReloadError::NoDirectories)));
        assert!(
            !*called.lock().unwrap(),
            "callback should not be called when no directories"
        );
    }

    #[test]
    fn test_nonexistent_directory_graceful() {
        let called = Arc::new(Mutex::new(false));
        let called_clone = Arc::clone(&called);
        let fake_path = PathBuf::from("/this/path/does/not/exist at all");
        let result = start_skill_watcher(
            vec![fake_path],
            Box::new(move || {
                *called_clone.lock().unwrap() = true;
            }),
        );
        // Should not panic, returns a handle even if dir doesn't exist
        assert!(result.is_ok());
        assert!(
            !*called.lock().unwrap(),
            "callback should not be called for nonexistent directory"
        );
    }

    #[test]
    fn test_callback_triggered_on_file_change() {
        let called = Arc::new(Mutex::new(0));
        let called_clone = Arc::clone(&called);
        let tmpdir = TempDir::new().unwrap();
        let dir = tmpdir.path().to_path_buf();

        let on_change = Box::new(move || {
            *called_clone.lock().unwrap() += 1;
        });

        let handle = start_skill_watcher(vec![dir.clone()], on_change).unwrap();

        // Write a file to trigger the watcher
        std::fs::write(dir.join("test_skill.md"), "# Test Skill").unwrap();

        // Wait long enough for debounce to settle
        std::thread::sleep(Duration::from_millis(400));

        assert!(
            *called.lock().unwrap() >= 1,
            "callback should be triggered at least once after file change"
        );

        drop(handle);
    }

    #[test]
    fn test_debounce_behavior() {
        let call_count = Arc::new(Mutex::new(0));
        let call_count_clone = Arc::clone(&call_count);
        let tmpdir = TempDir::new().unwrap();
        let dir = tmpdir.path().to_path_buf();

        let on_change = Box::new(move || {
            *call_count_clone.lock().unwrap() += 1;
        });

        let handle = start_skill_watcher(vec![dir.clone()], on_change).unwrap();

        // Rapidly write multiple files
        for i in 0..5 {
            std::fs::write(dir.join(format!("skill_{}.md", i)), "# Skill").unwrap();
        }

        // Wait for debounce window to close
        std::thread::sleep(Duration::from_millis(400));

        // With 300ms debounce, rapid writes should result in only 1 callback
        let count = *call_count.lock().unwrap();
        assert!(
            count <= 2,
            "debounce should coalesce rapid writes, got {} calls",
            count
        );

        drop(handle);
    }

    #[test]
    fn test_watcher_handle_drop_stops_watcher() {
        let tmpdir = TempDir::new().unwrap();
        let dir = tmpdir.path().to_path_buf();

        let called = Arc::new(Mutex::new(false));
        let called_clone = Arc::clone(&called);

        let handle = start_skill_watcher(
            vec![dir.clone()],
            Box::new(move || {
                *called_clone.lock().unwrap() = true;
            }),
        )
        .unwrap();

        drop(handle);

        // Write after dropping handle - callback should NOT fire
        std::fs::write(dir.join("after_drop.md"), "# After").unwrap();
        std::thread::sleep(Duration::from_millis(400));

        assert!(
            !*called.lock().unwrap(),
            "callback should not fire after handle is dropped"
        );
    }
}
