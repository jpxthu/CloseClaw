//! Integration tests: daemon shutdown timeouts read from system.json config.
//!
//! Cross-module direct calls against `closeclaw-config` public APIs
//! (`ConfigManager::new` / `reload_section` / `section`) — no full stack,
//! no daemon process (STANDARDS §1 integration classification).
//!
//! Migrated from `crates/daemon/src/daemon_shutdown_tests.rs` (issue #3244):
//! the in-crate fixture primitives (`load_system_config_manager`,
//! `temp_config_dir`) are `cfg(test)`-private to the daemon crate, but the
//! sequence is only 3 steps — write `system.json` → `ConfigManager::new` →
//! `reload_section(System)` — so it is inlined in [`system_fixture`].

use closeclaw_config::providers::SystemConfigData;
use closeclaw_config::{ConfigManager, ConfigSection};
use std::path::PathBuf;
use std::time::Duration;

/// Named-field fixture returned by [`system_fixture`]: `_guard` keeps the
/// temp config tree alive for the whole test — a named field, so the
/// keep-alive cannot be silently lost the way a wildcard tuple destructure
/// would — and `cm` is the reloaded manager under test.
struct SystemFixture {
    _guard: tempfile::TempDir,
    cm: ConfigManager,
}

/// Shared fixture for the config-driven shutdown-timeout tests below:
/// creates a temp config tree (`<tmp>/config/system.json` written from
/// `system_json`), builds a `ConfigManager`, and reloads the System section.
/// Inlined from the daemon crate's `cfg(test)` helpers
/// (`temp_config_dir` + `load_system_config_manager`), which cannot be
/// reached cross-crate.
/// Returns a [`SystemFixture`]: bind the whole value (e.g. `let fixture = …`)
/// so `_guard` stays alive for the test; the reload expect message is
/// caller-supplied so each test keeps its own wording.
fn system_fixture(system_json: serde_json::Value, reload_expect: &str) -> SystemFixture {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let config_dir: PathBuf = tmp.path().join("config");
    std::fs::create_dir_all(&config_dir).expect("create config dir");

    // Inlined `load_system_config_manager`: write system.json →
    // ConfigManager::new → reload_section(System).
    std::fs::write(
        config_dir.join("system.json"),
        serde_json::to_string(&system_json).expect("serialize system.json"),
    )
    .expect("write system.json");
    let cm = ConfigManager::new(config_dir).expect("ConfigManager::new");
    cm.reload_section(ConfigSection::System, None)
        .expect(reload_expect);

    SystemFixture { _guard: tmp, cm }
}

/// Per-session graceful timeout reads from config when available.
/// Verifies the config-based timeout path (reading system.shutdown.gracefulTimeoutSecs).
#[test]
fn test_per_session_graceful_timeout_reads_from_config() {
    // Write system.json with custom shutdown config
    let system_json = serde_json::json!({
        "shutdown": {
            "drainTimeoutSecs": 15,
            "gracefulTimeoutSecs": 45
        }
    });
    let fixture = system_fixture(
        system_json,
        "reload system.json with shutdown timeouts succeeds",
    );

    // Read the timeout the same way phase_2_session_stop does
    let timeout = fixture
        .cm
        .section(ConfigSection::System)
        .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())
        .and_then(|sys| sys.shutdown.map(|s| s.graceful_timeout_secs))
        .map(Duration::from_secs);

    assert_eq!(
        timeout,
        Some(Duration::from_secs(45)),
        "per-session graceful timeout should read 45s from config"
    );
}

/// Per-session graceful timeout falls back to DEFAULT_GRACEFUL_TIMEOUT
/// when shutdown config is absent.
#[test]
fn test_per_session_graceful_timeout_fallback_to_default() {
    use closeclaw_session::llm_session::session_handles::DEFAULT_GRACEFUL_TIMEOUT;

    // Write system.json WITHOUT shutdown config
    let system_json = serde_json::json!({ "version": "1.0" });
    let fixture = system_fixture(
        system_json,
        "reload system.json without shutdown config succeeds",
    );

    // Read the timeout the same way phase_2_session_stop does
    let timeout = fixture
        .cm
        .section(ConfigSection::System)
        .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())
        .and_then(|sys| sys.shutdown.map(|s| s.graceful_timeout_secs))
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_GRACEFUL_TIMEOUT);

    assert_eq!(
        timeout, DEFAULT_GRACEFUL_TIMEOUT,
        "should fall back to DEFAULT_GRACEFUL_TIMEOUT when config is absent"
    );
}

/// Drain timeout reads from config when available.
/// Verifies the config-based drain timeout path (reading system.shutdown.drainTimeoutSecs).
#[test]
fn test_drain_timeout_reads_from_config() {
    let system_json = serde_json::json!({
        "shutdown": {
            "drainTimeoutSecs": 20,
            "gracefulTimeoutSecs": 30
        }
    });
    let fixture = system_fixture(
        system_json,
        "reload system.json with drain timeout succeeds",
    );

    let drain_timeout = fixture
        .cm
        .section(ConfigSection::System)
        .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())
        .and_then(|sys| sys.shutdown.map(|s| s.drain_timeout_secs))
        .map(Duration::from_secs);

    assert_eq!(
        drain_timeout,
        Some(Duration::from_secs(20)),
        "drain timeout should read 20s from config"
    );
}
