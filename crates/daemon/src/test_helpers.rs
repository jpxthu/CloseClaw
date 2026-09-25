//! Shared test helpers for daemon tests.
//!
//! Duplicated from root crate's `common/test_helpers.rs` because the daemon
//! crate cannot depend on the root crate (circular dependency).

use std::io;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::processor::ContentBlock;
use closeclaw_config::manager::ConfigSection;
use closeclaw_config::ConfigManager;
use closeclaw_gateway::types::GatewayConfig;
use closeclaw_gateway::{Gateway, SessionManager};

use closeclaw_session::persistence::{
    DreamingStatus, PersistenceError, PersistenceService, SessionCheckpoint,
};
use tokio::sync::mpsc;

use crate::chat_rpc::{ChatContext, RpcTerminalPlugin};

/// Duplicate of `crate::bridge::common_shutdown_handle` for daemon-crate tests.
/// Creates a `closeclaw_gateway::shutdown_handle::ShutdownHandle` from the daemon's
/// `ShutdownHandle`.
pub fn common_shutdown_handle(
    daemon_handle: &crate::shutdown::ShutdownHandle,
) -> Arc<closeclaw_gateway::shutdown_handle::ShutdownHandle> {
    Arc::new(closeclaw_gateway::shutdown_handle::ShutdownHandle::new(
        Arc::new(daemon_handle.clone()),
    ))
}

/// Write a `models.json` defining `providers` into config dir `dir`.
///
/// Call after [`write_mandatory_configs`], which writes a placeholder
/// models.json that this helper overwrites.
pub fn write_models_providers(
    dir: &std::path::Path,
    providers: serde_json::Value,
) -> io::Result<()> {
    std::fs::write(
        dir.join("models.json"),
        serde_json::json!({ "mode": "merge", "providers": providers }).to_string(),
    )
}

/// Write a convention-directory credential file into
/// `<dir>/credentials/<provider>.json`.
pub fn write_provider_credential(
    dir: &std::path::Path,
    provider: &str,
    api_key: &str,
) -> io::Result<()> {
    let creds_dir = dir.join("credentials");
    std::fs::create_dir_all(&creds_dir)?;
    std::fs::write(
        creds_dir.join(format!("{}.json", provider)),
        serde_json::json!({ "provider": provider, "apiKey": api_key }).to_string(),
    )
}

/// Create and load a ConfigManager over config dir `dir` (the mandatory
/// config files must already exist — see [`write_mandatory_configs`]).
///
/// In tests `dir` plays the role of `<root>/config`.
pub fn load_config_manager(dir: &std::path::Path) -> ConfigManager {
    let cm = ConfigManager::new(dir.to_path_buf()).expect("ConfigManager::new");
    cm.load().expect("ConfigManager::load");
    cm
}

/// Create `<root>/config` with the mandatory skeleton written, and return a
/// loaded `Arc<ConfigManager>` over it — shared base for the per-file
/// `make_config_manager` test constructors (issue #3148).
///
/// The config subdir lives under `root`, matching the production layout
/// where agent directories resolve to `config_dir.parent()/agents`.
/// The new + load sequence matches [`load_config_manager`]; the dir
/// creation and skeleton write make it usable directly on a fresh
/// TempDir root. Mandatory files only — write extra files (e.g.
/// `session.json`) before calling this helper if `load` must see them.
///
/// Same name, three different meanings across this crate (issue #3245) —
/// pick by semantics:
/// - **this one** (crate-shared base): config dir is the `<root>/config`
///   subdir, the mandatory skeleton is written via
///   [`write_mandatory_configs`], then `load()` is called;
/// - `crate::session_config_provider_tests::make_config_manager`:
///   file-private wrapper around this one — when given `Some(session_json)`
///   it writes `session.json` into `<root>/config` **before** this
///   helper's `load()`;
/// - `crate::config_watcher::tests::make_config_manager`
///   (`crates/daemon/src/config_reload_tests.rs`, `pub(super)`): bare
///   variant — `config_dir` is the TempDir root itself, no skeleton write,
///   no `load()`.
pub fn make_config_manager(root: &std::path::Path) -> Arc<ConfigManager> {
    let config_dir = root.join("config");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    write_mandatory_configs(&config_dir).expect("mandatory configs");
    let cm = ConfigManager::new(config_dir).expect("ConfigManager::new");
    cm.load().expect("ConfigManager::load");
    Arc::new(cm)
}

/// Write the config skeleton into `dir`: the 5 mandatory files
/// (channels.json, gateway.json, plugins.json, system.json,
/// accounts.json) plus the optional models.json.
///
/// Delegates to the common single implementation (Step 1.20) — call
/// sites in this crate keep using this name.
pub fn write_mandatory_configs(dir: &std::path::Path) -> io::Result<()> {
    closeclaw_common::test_helpers::write_mandatory_configs(dir)
}

/// Four-step LLM registry fixture (Step 1.22 dedup): mandatory config
/// skeleton → models.json `providers` → convention-directory
/// credentials → `ConfigManager::load`.
///
/// Single point for the sequence shared by `llm_init_tests`,
/// `unit_tests` and `startup_tests` — tests only declare their
/// providers and credentials. `creds` entries are written as
/// `<dir>/credentials/<provider>.json`; pass `&[]` for no
/// convention-directory credentials (e.g. `credentialPath` or
/// env-fallback scenarios).
pub fn load_cm(
    dir: &std::path::Path,
    providers: serde_json::Value,
    creds: &[(&str, &str)],
) -> ConfigManager {
    write_mandatory_configs(dir).expect("mandatory configs");
    write_models_providers(dir, providers).expect("models.json");
    for &(provider, api_key) in creds {
        write_provider_credential(dir, provider, api_key).expect("credential file");
    }
    load_config_manager(dir)
}

/// Write `<config_dir>/system.json` from `system_json`, build a
/// [`ConfigManager`] over exactly `config_dir`, then reload **only** the
/// `System` section into it — shared system-section fixture primitive
/// (issue #3245), single definition of the sequence
/// "write system.json → new → reload_section(System)".
///
/// Preconditions: `config_dir` must exist (a `TempDir` root, or a subdir
/// the caller created such as `<tmp>/config`); other section files may be
/// absent, because only `System` is reloaded and `load()` is never called
/// (for the full mandatory skeleton see [`make_config_manager`] /
/// [`write_mandatory_configs`], a different-semantics helper).
///
/// Failure handling: serialization, the file write and
/// `ConfigManager::new` panic here; the `reload_section(System)` failure
/// panics with the caller-supplied `reload_expect` message, so each call
/// site keeps its own wording. The returned manager is owned — callers
/// wrap it in `Arc` or their own fixture struct and keep the `TempDir`
/// alive themselves.
pub fn load_system_config_manager(
    config_dir: &std::path::Path,
    system_json: serde_json::Value,
    reload_expect: &str,
) -> ConfigManager {
    std::fs::write(
        config_dir.join("system.json"),
        serde_json::to_string(&system_json).expect("serialize system.json"),
    )
    .expect("write system.json");
    let cm = ConfigManager::new(config_dir.to_path_buf()).expect("ConfigManager::new");
    cm.reload_section(ConfigSection::System, None)
        .expect(reload_expect);
    cm
}

// ── load_system_config_manager contract tests (issue #3245) ──────────────

/// Contract — happy path: the manager returned by the primitive has the
/// `System` section cached with the written value intact.
#[test]
fn test_load_system_config_manager_loads_system_section() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let system_json = serde_json::json!({
        "version": "1.0",
        "commands": { "ownerDisplay": "feishu:oc_contract" }
    });
    let cm = load_system_config_manager(tmp.path(), system_json, "reload succeeds");
    let system = cm
        .section(ConfigSection::System)
        .expect("System section must be cached after the primitive's reload");
    assert_eq!(
        system["commands"]["ownerDisplay"],
        serde_json::json!("feishu:oc_contract"),
        "System section must reflect the value the primitive wrote to system.json"
    );
}

/// Contract — layout variant: `config_dir` as the TempDir root (the
/// config_reload_tests caller layout) exposes exactly the written value.
#[test]
fn test_load_system_config_manager_root_layout() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let cm = load_system_config_manager(
        tmp.path(),
        serde_json::json!({ "version": "1.0", "source": "root-layout" }),
        "reload succeeds",
    );
    assert_eq!(
        cm.section(ConfigSection::System).expect("System section"),
        serde_json::json!({ "version": "1.0", "source": "root-layout" }),
        "root-layout config_dir must expose exactly the written system.json"
    );
    // Only System is reloaded — no other section may silently appear (the
    // primitive must not run a full load(), which would need the skeleton).
    assert_eq!(
        cm.section(ConfigSection::Gateway),
        None,
        "primitive reloads only System; Gateway must stay absent"
    );
}

/// Contract — layout variant: `config_dir` as a `<tmp>/config` subdir (the
/// daemon_shutdown_tests caller layout) exposes exactly the written value.
#[test]
fn test_load_system_config_manager_config_subdir_layout() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let config_subdir = tmp.path().join("config");
    std::fs::create_dir_all(&config_subdir).expect("create <tmp>/config");
    let cm = load_system_config_manager(
        &config_subdir,
        serde_json::json!({ "version": "1.0", "source": "subdir-layout" }),
        "reload succeeds",
    );
    assert_eq!(
        cm.section(ConfigSection::System).expect("System section"),
        serde_json::json!({ "version": "1.0", "source": "subdir-layout" }),
        "<tmp>/config layout must expose exactly the written system.json"
    );
}

/// Contract — error path: a `reload_section(System)` failure panics with
/// the **caller-supplied** `reload_expect` message (the branch call sites
/// rely on), not with one of the primitive's own expect labels.
///
/// The primitive serializes `system_json` itself, so the file it writes is
/// always valid JSON — parse failures are unreachable by construction. The
/// reachable failure mode is I/O: `fs::write` needs only the write bit,
/// while the reload's `read_to_string` needs the read bit, so a
/// **write-only** (mode 0o200) pre-existing `system.json` is overwritten by
/// the primitive yet fails to read back → `ConfigLoadError::IoError` → the
/// `reload_expect` panic asserted below.
#[test]
#[should_panic(expected = "reload failure must surface the caller-supplied message")]
fn test_load_system_config_manager_reload_failure_uses_caller_message() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::TempDir::new().expect("temp dir");
    let seeded = tmp.path().join("system.json");
    std::fs::write(&seeded, r#"{"version":"1.0"}"#).expect("seed system.json");
    let mut perms = std::fs::metadata(&seeded)
        .expect("stat system.json")
        .permissions();
    perms.set_mode(0o200); // owner-write-only: writable for fs::write, unreadable for reload
    std::fs::set_permissions(&seeded, perms).expect("make system.json write-only");
    load_system_config_manager(
        tmp.path(),
        serde_json::json!({ "version": "1.0" }),
        "reload failure must surface the caller-supplied message",
    );
}

// ── Turn-completion consumer test harness ─────────────────────────────────

/// Shared setup result for turn-completion consumer tests
/// (`chat_rpc_tests` + `gateway_restart_tests`, Step 1.20 dedup): one
/// waiting chat connection registered on a fresh plugin (conn 1 + agent
/// route `"master"`) and the `SessionMessageHandler` output channel
/// wired into the shared consumer
/// [`crate::chat_rpc::spawn_turn_completion_consumer`] (the assembly
/// point both production paths call). Payload and assertions stay with
/// each test.
///
/// The consumer task holds an `Arc<RpcTerminalPlugin>`, keeping the
/// registered connection sender alive until `output_tx` is dropped —
/// tests need not hold the plugin itself.
pub struct TurnCompletionHarness {
    /// Receiver for the waiting connection (closes on `finish_turns`).
    pub conn_rx: mpsc::Receiver<RenderedOutput>,
    /// Sender for `SessionMessageHandler` output messages.
    pub output_tx: mpsc::Sender<(String, Vec<ContentBlock>)>,
    /// The shared consumer task (await after dropping `output_tx`).
    pub consumer: tokio::task::JoinHandle<()>,
}

/// Build the [`TurnCompletionHarness`]: waiting connection + wired
/// consumer.
pub async fn setup_turn_completion_consumer() -> TurnCompletionHarness {
    let plugin = Arc::new(RpcTerminalPlugin::new());
    let (conn_tx, conn_rx) = mpsc::channel(4);
    plugin.register_sender(1, conn_tx).await;
    plugin.register_agent_route("master", 1).await;
    let (output_tx, output_rx) = mpsc::channel(64);
    let consumer = crate::chat_rpc::spawn_turn_completion_consumer(output_rx, Arc::clone(&plugin));
    TurnCompletionHarness {
        conn_rx,
        output_tx,
        consumer,
    }
}

// ── Shared TestStorage ───────────────────────────────────────────────────

/// Minimal in-memory [`PersistenceService`] for unit tests.
#[derive(Debug, Default)]
pub struct TestStorage {
    /// Active / general checkpoints.
    pub checkpoints: Mutex<Vec<SessionCheckpoint>>,
    /// Archived checkpoints (used by `list_archived_unmined_sessions`).
    pub archived: Mutex<Vec<SessionCheckpoint>>,
    /// Tracks which sessions were marked mined (for assertion in tests).
    pub mined_ids: Mutex<Vec<String>>,
}

impl TestStorage {
    /// Insert a checkpoint into the active store.
    pub fn add_checkpoint(&self, cp: SessionCheckpoint) {
        self.checkpoints.lock().unwrap().push(cp);
    }

    /// Insert a checkpoint into the archived store.
    pub fn add_archived(&self, cp: SessionCheckpoint) {
        self.archived.lock().unwrap().push(cp);
    }

    /// Return a clone of the mined session IDs recorded so far.
    pub fn mined_ids(&self) -> Vec<String> {
        self.mined_ids.lock().unwrap().clone()
    }
}

#[async_trait]
impl PersistenceService for TestStorage {
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), PersistenceError> {
        self.checkpoints.lock().unwrap().push(checkpoint.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self
            .checkpoints
            .lock()
            .unwrap()
            .iter()
            .find(|cp| cp.session_id == session_id)
            .cloned())
    }

    async fn load_archived_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self
            .archived
            .lock()
            .unwrap()
            .iter()
            .find(|cp| cp.session_id == session_id)
            .cloned())
    }

    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        self.checkpoints
            .lock()
            .unwrap()
            .retain(|cp| cp.session_id != session_id);
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_archived_unmined_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(self
            .archived
            .lock()
            .unwrap()
            .iter()
            .filter(|cp| !cp.mined)
            .map(|cp| cp.session_id.clone())
            .collect())
    }

    async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let cps = self.checkpoints.lock().unwrap();
        Ok(cps
            .iter()
            .filter(|cp| cp.mined && cp.dreaming_status != DreamingStatus::Completed)
            .map(|cp| cp.session_id.clone())
            .collect())
    }

    async fn mark_mined(&self, session_id: &str) -> Result<(), PersistenceError> {
        self.mined_ids.lock().unwrap().push(session_id.into());
        Ok(())
    }

    async fn update_dreaming_status(
        &self,
        session_id: &str,
        status: DreamingStatus,
    ) -> Result<(), PersistenceError> {
        let mut cps = self.checkpoints.lock().unwrap();
        if let Some(cp) = cps.iter_mut().find(|cp| cp.session_id == session_id) {
            cp.dreaming_status = status;
        }
        Ok(())
    }
}

// ── Shared dispatch context factory ───────────────────────────────────────

/// Test cap substituted for the default `max_message_size` = 0 by
/// [`effective_test_config`]; keeps inbound validation from rejecting
/// non-empty test messages.
pub(crate) const TEST_MAX_MESSAGE_SIZE: usize = 64 * 1024;

/// Normalize a test [`GatewayConfig`] for inbound validation: at the config
/// default of `max_message_size` = 0 any non-empty text message exceeds the
/// limit and is rejected by `validate_inbound`, so a sentinel 0 is replaced
/// by [`TEST_MAX_MESSAGE_SIZE`]; an explicit non-zero cap passes through
/// unchanged.
pub(crate) fn effective_test_config(config: GatewayConfig) -> GatewayConfig {
    if config.max_message_size == 0 {
        GatewayConfig {
            max_message_size: TEST_MAX_MESSAGE_SIZE,
            ..config
        }
    } else {
        config
    }
}

/// Sentinel contract — the default config (`max_message_size` = 0) gets the
/// test cap so `validate_inbound` accepts non-empty text.
#[test]
fn test_effective_test_config_fills_default_sentinel() {
    let config = effective_test_config(GatewayConfig::default());
    assert_eq!(config.max_message_size, TEST_MAX_MESSAGE_SIZE);
}

/// Sentinel contract — an explicit non-zero cap is kept as-is.
#[test]
fn test_effective_test_config_keeps_explicit_cap() {
    let config = GatewayConfig {
        max_message_size: 4096,
        ..Default::default()
    };
    let effective = effective_test_config(config);
    assert_eq!(effective.max_message_size, 4096);
}

/// Shared base harness for the dispatch tests (issue #3067):
/// `SessionManager` + `Gateway` + `RpcTerminalPlugin` assembled into a
/// [`ChatContext`]. Callers may pass `GatewayConfig::default()` and still
/// be safe for inbound validation — [`effective_test_config`] substitutes
/// the cap.
pub(crate) fn make_dispatch_context(config: GatewayConfig) -> ChatContext {
    let config = effective_test_config(config);
    let sessions = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    let gateway = Arc::new(Gateway::new(config, sessions));
    let rpc_plugin = Arc::new(RpcTerminalPlugin::new());
    ChatContext {
        gateway,
        rpc_plugin,
    }
}

/// Graceful-shutdown signals a test may deliver to its own process.
///
/// Test-only type carrying the SIGTERM/SIGINT-only invariant of
/// [`kill_self`] in code instead of a caller convention: every variant is
/// one of the two graceful first-signal shutdowns defined by
/// `docs/design/daemon/shutdown.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestShutdownSignal {
    /// SIGTERM — graceful shutdown trigger.
    Sigterm,
    /// SIGINT — graceful shutdown trigger.
    Sigint,
}

impl TestShutdownSignal {
    /// The libc signal number this variant delivers.
    pub(crate) fn as_libc_signal(self) -> libc::c_int {
        match self {
            TestShutdownSignal::Sigterm => libc::SIGTERM,
            TestShutdownSignal::Sigint => libc::SIGINT,
        }
    }
}

/// Send shutdown signal `sig` to the current process itself.
///
/// Single point wrapping the libc kill call for daemon tests — the signal
/// is chosen through [`TestShutdownSignal`], so the type, not caller
/// convention, limits delivery to the graceful SIGTERM/SIGINT pair.
pub(crate) fn kill_self(sig: TestShutdownSignal) {
    // SAFETY: the target pid is `std::process::id()`, i.e. this process
    // itself, so the signal is delivered only to the calling process and
    // never to another one; `sig` is a `TestShutdownSignal`, whose only
    // variants are the graceful SIGTERM/SIGINT shutdown signals, so the
    // delivered signal's effect on this process is exactly what the
    // surrounding test exercises.
    //
    // The return value is checked so a failed kill surfaces the OS error
    // immediately instead of letting the test hang on the un-sent signal.
    let ret = unsafe { libc::kill(std::process::id() as libc::pid_t, sig.as_libc_signal()) };
    assert_eq!(
        ret,
        0,
        "kill self failed: {}",
        std::io::Error::last_os_error()
    );
}
