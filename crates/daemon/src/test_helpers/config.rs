//! Config / ConfigManager test fixtures: mandatory skeleton writes,
//! loaded-manager constructors and the system-section reload primitive.

use std::io;
use std::sync::Arc;

use closeclaw_config::manager::ConfigSection;
use closeclaw_config::ConfigManager;

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

/// Whether the current process runs as root, read from `/proc/self/status`
/// (no libc dependency) — same precedent as
/// `crates/tools/src/builtin/readback_tests.rs`.
fn is_root() -> bool {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:\t") {
            if let Some(uid_str) = rest.split('\t').next() {
                return uid_str == "0";
            }
        }
    }
    false
}

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
///
/// Root guard: root bypasses DAC, so the 0o200 file stays readable, the
/// reload succeeds and nothing panics — the test skips itself under root
/// (same `is_root()` precedent as `crates/tools/src/builtin/readback_tests.rs`).
/// A `#[should_panic]` attribute cannot skip (an early return would *fail*
/// the attribute), so the panic is caught and asserted manually instead.
#[test]
fn test_load_system_config_manager_reload_failure_uses_caller_message() {
    use std::os::unix::fs::PermissionsExt;

    if is_root() {
        eprintln!("SKIP: root ignores permission bits (0o200 stays readable)");
        return;
    }

    let tmp = tempfile::TempDir::new().expect("temp dir");
    let seeded = tmp.path().join("system.json");
    std::fs::write(&seeded, r#"{"version":"1.0"}"#).expect("seed system.json");
    let mut perms = std::fs::metadata(&seeded)
        .expect("stat system.json")
        .permissions();
    perms.set_mode(0o200); // owner-write-only: writable for fs::write, unreadable for reload
    std::fs::set_permissions(&seeded, perms).expect("make system.json write-only");

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        load_system_config_manager(
            tmp.path(),
            serde_json::json!({ "version": "1.0" }),
            "reload failure must surface the caller-supplied message",
        );
    }))
    .expect_err("a failed reload_section(System) must panic");
    let msg = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("<non-string panic payload>");
    assert!(
        msg.contains("reload failure must surface the caller-supplied message"),
        "the panic must carry the caller-supplied reload_expect message, got: {msg}"
    );
}
