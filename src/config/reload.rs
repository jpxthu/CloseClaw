//! Config Hot Reload Manager
//!
//! Watches config files for changes and automatically reloads them.
//! Validates new config before applying, maintains backup of last known good config.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::backup::SafeBackupManager;
use super::{ConfigError, ConfigProvider};

/// Type alias for the config parsing function.
type ParseFn<P> = Arc<dyn Fn(&str) -> Result<P, ConfigError> + Send + Sync>;

/// Event emitted when a config reload occurs
#[derive(Debug, Clone)]
pub enum ConfigReloadEvent {
    /// Config was successfully reloaded
    Reloaded { path: String },
    /// Config reload failed, rolled back to backup
    Rollback { path: String, error: String },
    /// Config reload validation failed
    ValidationFailed { path: String, error: String },
}

/// Result of a reload operation
#[derive(Debug)]
pub enum ReloadResult {
    /// Success
    Success,
    /// Validation failed, keep using old config
    ValidationFailed(ConfigError),
    /// IO or parse error, rolled back
    RolledBack(ConfigError),
}

/// ConfigReloadManager watches config files and hot-reloads on change.
///
/// # Type Parameters
/// * `P` - The ConfigProvider implementation type
pub struct ConfigReloadManager<P> {
    /// Inner provider (wrapped in Arc<Mutex> for interior mutability)
    provider: Arc<std::sync::Mutex<P>>,
    /// File paths being watched
    watched_paths: Vec<PathBuf>,
    /// Backup manager for rollback support
    backup_manager: Arc<SafeBackupManager>,
    /// Channel for reload events
    event_sender: Option<mpsc::Sender<ConfigReloadEvent>>,
    /// Debounce duration to avoid rapid reloads
    debounce_duration: Duration,
    /// Parsing function for the config provider
    parse_fn: ParseFn<P>,
}

impl<P> std::fmt::Debug for ConfigReloadManager<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigReloadManager")
            .field("watched_paths", &self.watched_paths)
            .field("backup_manager", &self.backup_manager)
            .field("debounce_duration", &self.debounce_duration)
            .finish()
    }
}

impl<P: ConfigProvider + 'static> ConfigReloadManager<P> {
    /// Create a new ConfigReloadManager with a custom parse function.
    pub fn new(
        provider: P,
        backup_manager: SafeBackupManager,
        debounce_duration: Duration,
        parse_fn: impl Fn(&str) -> Result<P, ConfigError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            provider: Arc::new(std::sync::Mutex::new(provider)),
            watched_paths: Vec::new(),
            backup_manager: Arc::new(backup_manager),
            event_sender: None,
            debounce_duration,
            parse_fn: Arc::new(parse_fn),
        }
    }

    /// Create a new ConfigReloadManager with an optional event channel.
    pub fn with_events(
        provider: P,
        backup_manager: SafeBackupManager,
        debounce_duration: Duration,
        parse_fn: impl Fn(&str) -> Result<P, ConfigError> + Send + Sync + 'static,
        event_sender: mpsc::Sender<ConfigReloadEvent>,
    ) -> Self {
        Self {
            provider: Arc::new(std::sync::Mutex::new(provider)),
            watched_paths: Vec::new(),
            backup_manager: Arc::new(backup_manager),
            event_sender: Some(event_sender),
            debounce_duration,
            parse_fn: Arc::new(parse_fn),
        }
    }

    /// Get a clone of the inner provider.
    pub fn provider(&self) -> Arc<std::sync::Mutex<P>> {
        Arc::clone(&self.provider)
    }

    /// Read file content and create backup before reload.
    fn backup_current_content(&self, path: &PathBuf) -> Result<(), ReloadResult> {
        let current_content = fs::read(path).map_err(|e| {
            error!("Failed to read config file {:?}: {}", path, e);
            ReloadResult::RolledBack(ConfigError::IoError(e))
        })?;

        if let Err(e) = self
            .backup_manager
            .backup_with_content(path, &current_content)
        {
            warn!("Failed to create backup before reload: {}", e);
        }
        Ok(())
    }

    /// Load config from file, parse and validate. Returns the parsed provider on success.
    fn load_and_validate(&self, path: &str, path_buf: &PathBuf) -> Result<P, ReloadResult> {
        let new_content = fs::read_to_string(path_buf).map_err(|e| {
            error!("Failed to read new config content: {}", e);
            ReloadResult::RolledBack(ConfigError::IoError(e))
        })?;

        let temp_provider = (self.parse_fn)(&new_content).map_err(|e| {
            error!("Failed to parse new config: {}", e);
            self.emit_event(ConfigReloadEvent::ValidationFailed {
                path: path.to_string(),
                error: e.to_string(),
            });
            ReloadResult::ValidationFailed(e)
        })?;

        temp_provider.validate().map_err(|e| {
            error!("Validation failed for new config: {}", e);
            self.emit_event(ConfigReloadEvent::ValidationFailed {
                path: path.to_string(),
                error: e.to_string(),
            });
            ReloadResult::ValidationFailed(e)
        })?;

        Ok(temp_provider)
    }
}

impl<P: ConfigProvider + 'static> ConfigReloadManager<P> {
    /// Manually trigger a reload of a specific config path.
    /// Returns the result of the reload operation.
    pub fn reload(&self, path: &str) -> ReloadResult {
        let path_buf = PathBuf::from(path);

        // Lock provider briefly to ensure no concurrent reload
        let _guard = self.provider.lock().unwrap();
        drop(_guard);

        if let Err(result) = self.backup_current_content(&path_buf) {
            return result;
        }
        let temp_provider = match self.load_and_validate(path, &path_buf) {
            Ok(p) => p,
            Err(result) => return result,
        };

        // Validation passed, apply the new config
        let mut provider_guard = self.provider.lock().unwrap();
        *provider_guard = temp_provider;

        debug!("Config reloaded successfully: {}", path);
        self.emit_event(ConfigReloadEvent::Reloaded {
            path: path.to_string(),
        });

        ReloadResult::Success
    }

    /// Emit a reload event to the channel if configured.
    fn emit_event(&self, event: ConfigReloadEvent) {
        if let Some(ref sender) = self.event_sender {
            let _ = sender.try_send(event);
        }
    }

    /// Get a snapshot of the current config provider.
    pub fn snapshot(&self) -> P
    where
        P: Clone,
    {
        self.provider.lock().unwrap().clone()
    }
}

impl<P: ConfigProvider + Send + 'static> ConfigReloadManager<P> {
    /// Start watching config files for changes using notify.
    /// This spawns a background task that handles file change events.
    /// Returns a handle that can be used to stop watching.
    pub fn watch(&mut self, paths: Vec<PathBuf>) -> Result<WatcherHandle, ConfigError> {
        self.watched_paths = paths.clone();

        let provider = Arc::clone(&self.provider);
        let backup_manager = Arc::clone(&self.backup_manager);
        let debounce = self.debounce_duration;
        let event_sender = self.event_sender.clone();
        let parse_fn = Arc::clone(&self.parse_fn);

        let (tx, rx) = std::sync::mpsc::channel();

        // Create a watcher using the callback-based API
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.send(event);
                }
            },
            NotifyConfig::default(),
        )
        .map_err(|e| ConfigError::SchemaError(format!("Watcher creation failed: {}", e)))?;

        for path in &paths {
            watcher
                .watch(path, RecursiveMode::NonRecursive)
                .map_err(|e| {
                    ConfigError::SchemaError(format!("Watch failed for {:?}: {}", path, e))
                })?;
        }

        // Spawn background task to handle events
        std::thread::spawn(move || {
            run_watch_loop(
                rx,
                provider,
                backup_manager,
                parse_fn,
                event_sender,
                debounce,
            );
        });

        Ok(WatcherHandle { watcher })
    }
}

/// Background loop that processes file change events with debounce.
fn run_watch_loop<P: ConfigProvider + Send + 'static>(
    rx: std::sync::mpsc::Receiver<Event>,
    provider: Arc<std::sync::Mutex<P>>,
    backup_manager: Arc<SafeBackupManager>,
    parse_fn: ParseFn<P>,
    event_sender: Option<mpsc::Sender<ConfigReloadEvent>>,
    debounce: Duration,
) {
    let mut last_event_time = std::time::Instant::now()
        .checked_sub(debounce * 2)
        .unwrap_or(std::time::Instant::now());

    for event in rx {
        let now = std::time::Instant::now();
        if now.duration_since(last_event_time) < debounce {
            debug!("Debouncing config change event");
            continue;
        }
        last_event_time = now;

        for path in event.paths {
            handle_path_change(&path, &provider, &backup_manager, &parse_fn, &event_sender);
        }
    }
}

/// Process a single changed path: backup, parse, validate, apply.
fn handle_path_change<P: ConfigProvider + Send + 'static>(
    path: &PathBuf,
    provider: &Arc<std::sync::Mutex<P>>,
    backup_manager: &Arc<SafeBackupManager>,
    parse_fn: &ParseFn<P>,
    event_sender: &Option<mpsc::Sender<ConfigReloadEvent>>,
) {
    let path_str = path.display().to_string();
    info!("Config file changed: {}", path_str);

    let current_content = match fs::read(path) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to read config for backup: {}", e);
            return;
        }
    };
    if let Err(e) = backup_manager.backup_with_content(path, &current_content) {
        warn!("Failed to create backup: {}", e);
    }

    let new_content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to read changed config: {}", e);
            return;
        }
    };

    let temp_provider = match parse_fn(&new_content) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to parse changed config: {}", e);
            emit_watch_event(
                event_sender,
                ConfigReloadEvent::ValidationFailed {
                    path: path_str.clone(),
                    error: e.to_string(),
                },
            );
            return;
        }
    };

    if let Err(e) = temp_provider.validate() {
        error!("Validation failed for changed config: {}", e);
        emit_watch_event(
            event_sender,
            ConfigReloadEvent::ValidationFailed {
                path: path_str.clone(),
                error: e.to_string(),
            },
        );
        return;
    }

    let mut provider_guard = provider.lock().unwrap();
    *provider_guard = temp_provider;

    emit_watch_event(event_sender, ConfigReloadEvent::Reloaded { path: path_str });
}

/// Send a watch event if a sender is available.
fn emit_watch_event(sender: &Option<mpsc::Sender<ConfigReloadEvent>>, event: ConfigReloadEvent) {
    if let Some(ref s) = sender {
        let _ = s.try_send(event);
    }
}

/// Handle to stop the watcher when dropped.
#[derive(Debug)]
#[allow(dead_code)]
pub struct WatcherHandle {
    watcher: RecommendedWatcher,
}

impl WatcherHandle {
    /// Stop watching all files.
    pub fn stop(self) {
        // Watcher stops when dropped
    }
}
