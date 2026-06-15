//! Config Hot Reload Initialization
//!
//! Initializes file watching for configuration changes at daemon startup.
//! Watches individual config JSON files and the agents/ directory, triggering
//! incremental or full reloads on the ConfigManager.

use crate::config::manager::{ConfigManager, ConfigSection};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::info;

/// RAII handle for the config file watcher.
///
/// Dropping this stops the underlying filesystem watcher.
pub(crate) struct ConfigWatcherHandle {
    _watcher: RecommendedWatcher,
}

/// Map a config filename to its corresponding ConfigSection.
///
/// `agents.json` is NOT mapped here — it is handled separately via
/// `ConfigManager::reload_agents()` in the event consumer.
fn filename_to_section(filename: &str) -> Option<ConfigSection> {
    match filename {
        "models.json" => Some(ConfigSection::Models),
        "channels.json" => Some(ConfigSection::Channels),
        "gateway.json" => Some(ConfigSection::Gateway),
        "plugins.json" => Some(ConfigSection::Plugins),
        "system.json" => Some(ConfigSection::System),
        _ => None,
    }
}

/// Create a filesystem watcher and return it with the event receiver.
///
/// Watches individual config JSON files and the `agents/` directory.
fn setup_watcher(
    config_dir: &str,
) -> anyhow::Result<(RecommendedWatcher, mpsc::Receiver<notify::Result<Event>>)> {
    let config_path = Path::new(config_dir);
    let agents_dir = config_path.join("agents");

    let agents_json_path = config_path.join("config").join("agents.json");

    let config_files: Vec<PathBuf> = [
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
    ]
    .iter()
    .map(|f| config_path.join(f))
    .chain(std::iter::once(agents_json_path))
    .collect();

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>(64);

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                let _ = tx.try_send(Ok(event));
            }
        },
        notify::Config::default(),
    )?;

    for path in &config_files {
        if path.exists() {
            watcher.watch(path.as_ref(), RecursiveMode::NonRecursive)?;
        }
    }

    if agents_dir.exists() {
        watcher.watch(agents_dir.as_ref(), RecursiveMode::Recursive)?;
    }

    Ok((watcher, rx))
}

/// Reload all agent configs and log the result.
fn reload_agents_with_log(path: &std::path::Path, cm: &ConfigManager) {
    info!(
        path = %path.display(),
        "agent config change detected, reloading agents"
    );
    if let Err(e) = cm.reload_agents() {
        tracing::warn!(error = %e, "failed to reload agent configs");
    }
}

/// Handle a single changed file path: reload agents or the matching section.
async fn handle_changed_path(path: &std::path::Path, cm: &ConfigManager) {
    let path_str = path.to_string_lossy();

    if path_str.contains("/agents/") || path_str.contains("\\agents\\") {
        reload_agents_with_log(path, cm);
        return;
    }

    let filename = match path.file_name().and_then(|n| n.to_str()) {
        Some(f) => f,
        None => return,
    };

    if filename == "agents.json" {
        reload_agents_with_log(path, cm);
        return;
    }

    if let Some(section) = filename_to_section(filename) {
        match tokio::fs::read_to_string(path).await {
            Ok(content) => {
                info!(
                    path = %path.display(),
                    section = %section,
                    "config file changed, reloading section"
                );
                if let Err(e) = cm.reload_section(section, &content) {
                    tracing::warn!(
                        error = %e, section = %section,
                        "failed to reload config section"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e, path = %path.display(),
                    "failed to read changed config file"
                );
            }
        }
    }
}

/// Spawn the background event consumer that processes file change events.
///
/// Applies a simple time-window debounce and dispatches reloads to the
/// appropriate `ConfigManager` method based on the changed path.
fn spawn_event_consumer(
    mut rx: mpsc::Receiver<notify::Result<Event>>,
    config_manager: Arc<ConfigManager>,
) {
    tokio::spawn(async move {
        let debounce = Duration::from_millis(500);
        let mut last_reload = Instant::now() - debounce * 2;

        while let Some(event_result) = rx.recv().await {
            let event = match event_result {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "config watcher event error");
                    continue;
                }
            };

            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {}
                _ => continue,
            }

            let now = Instant::now();
            if now.duration_since(last_reload) < debounce {
                continue;
            }
            last_reload = now;

            for path in &event.paths {
                handle_changed_path(path, &config_manager).await;
            }
        }
    });
}

/// Initialize config hot-reload: create a file watcher and spawn the event consumer.
///
/// Returns the watcher handle (RAII: stops on drop). The spawned tokio task
/// continues running in the background for the lifetime of the daemon.
pub(crate) fn init_config_hot_reload(
    config_dir: &str,
    config_manager: Arc<ConfigManager>,
) -> anyhow::Result<ConfigWatcherHandle> {
    let (watcher, rx) = setup_watcher(config_dir)?;
    spawn_event_consumer(rx, Arc::clone(&config_manager));

    let config_path = Path::new(config_dir);
    let agents_dir = config_path.join("agents");
    let agents_json_path = config_path.join("config").join("agents.json");
    let watch_count = [
        "models.json",
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
    ]
    .iter()
    .filter(|f| config_path.join(f).exists())
    .count()
        + if agents_json_path.exists() { 1 } else { 0 }
        + if agents_dir.exists() { 1 } else { 0 };

    info!(
        watch_count,
        "config hot-reload initialized, watching {} \
         files + agents/ directory",
        watch_count,
    );

    Ok(ConfigWatcherHandle { _watcher: watcher })
}

#[cfg(test)]
#[path = "config_reload_tests.rs"]
mod tests;
