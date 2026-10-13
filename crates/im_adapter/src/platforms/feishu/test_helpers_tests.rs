//! Self-tests for the shared mock-script helpers (`test_helpers`).
//!
//! Convention (see `test_helpers` module docs): any test that creates and
//! actually spawns a mock CLI script must be marked `#[serial_test::serial]`
//! (ETXTBSY root cause); mock scripts must live in a `tempfile::TempDir`.

use super::{
    create_failing_mock_cli, make_adapter, make_adapter_with_profile, make_adapter_with_store,
    make_plugin, make_plugin_with_store, make_test_media_store, make_test_media_store_at,
    make_test_media_store_with_tmp, write_mock_script, CAPABILITY_ERROR_JSON,
};
use serial_test::serial;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

#[cfg(unix)]
fn assert_executable(path: &std::path::Path) {
    let mode = std::fs::metadata(path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755, "script must be executable (0755)");
}

#[test]
fn test_write_mock_script_writes_content_to_existing_executable_path() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("normal.sh");
    let content = "#!/bin/bash\necho 'hello'\n";

    write_mock_script(&path, content);

    assert!(path.exists(), "returned target path must exist");
    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(written, content, "file content must match input verbatim");
    #[cfg(unix)]
    assert_executable(&path);
}

#[test]
fn test_write_mock_script_overwrites_existing_target_without_tmp_leftover() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("overwrite.sh");

    write_mock_script(&path, "#!/bin/bash\necho 'old'\n");
    write_mock_script(&path, "#!/bin/bash\necho 'new'\n");

    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(written, "#!/bin/bash\necho 'new'\n", "old content replaced");
    assert!(
        !path.with_extension("sh.tmp").exists(),
        "no .tmp residue left next to the target"
    );
}

#[test]
fn test_failing_creator_places_executable_script_under_given_name() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp, "failing_lark_cli.sh", r#"{"code":1}"#, false, 1);

    let path = std::path::PathBuf::from(&cli);
    assert_eq!(path, tmp.path().join("failing_lark_cli.sh"));
    assert!(path.exists());
    #[cfg(unix)]
    assert_executable(&path);
}

#[test]
fn test_failing_creator_emits_error_json_on_stderr_only() {
    let tmp = TempDir::new().unwrap();
    let json = r#"{"code":230001,"msg":"capability error"}"#;
    let cli = create_failing_mock_cli(&tmp, "stderr_only.sh", json, false, 1);

    let content = std::fs::read_to_string(&cli).unwrap();
    assert!(
        content.contains(&format!("echo '{json}' >&2")),
        "error JSON must be echoed to stderr"
    );
    assert!(
        !content
            .lines()
            .any(|l| l.starts_with("echo") && !l.contains(">&2")),
        "should_echo_stdout=false must not add a stdout echo line"
    );
}

#[test]
fn test_failing_creator_stdout_echo_follows_flag() {
    let tmp = TempDir::new().unwrap();
    let json = r#"{"code":2}"#;

    let loud = create_failing_mock_cli(&tmp, "echo_on.sh", json, true, 1);
    let loud_content = std::fs::read_to_string(&loud).unwrap();
    assert!(
        loud_content.contains(&format!("echo '{json}'\n")),
        "should_echo_stdout=true must echo the same JSON to stdout"
    );

    let quiet = create_failing_mock_cli(&tmp, "echo_off.sh", json, false, 1);
    let quiet_content = std::fs::read_to_string(&quiet).unwrap();
    assert_eq!(
        quiet_content.matches(&format!("echo '{json}'")).count(),
        1,
        "should_echo_stdout=false must emit the JSON exactly once (stderr only)"
    );
}

#[test]
fn test_failing_creator_bakes_exit_code_into_script() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp, "exit_code.sh", r#"{"code":3}"#, false, 42);

    let content = std::fs::read_to_string(&cli).unwrap();
    assert!(content.contains("exit 42"), "script must exit with 42");
    assert!(!content.contains("exit 1"), "no stray default exit code");
}

/// End-to-end: the generated script is a real executable whose runtime
/// behavior matches the spec (stderr JSON + stdout echo + exit code).
#[serial]
#[test]
fn test_failing_creator_spawned_script_matches_spec() {
    let tmp = TempDir::new().unwrap();
    let json = r#"{"code":9,"msg":"boom"}"#;
    let cli = create_failing_mock_cli(&tmp, "spawn_spec.sh", json, true, 3);

    let output = std::process::Command::new(&cli).output().unwrap();
    assert_eq!(output.status.code(), Some(3), "exit code must propagate");
    assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), json);
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), json);
}

#[test]
fn test_make_adapter_overrides_cli_command_and_uses_test_profile() {
    let adapter = make_adapter("my_lark_cli");

    assert_eq!(
        adapter.cli_command, "my_lark_cli",
        "cli_command must equal the passed value"
    );
    assert_eq!(adapter.profile, "test_profile");
}

#[test]
fn test_make_plugin_builds_plugin_with_wired_adapter() {
    let plugin = make_plugin("wired_cli.sh");

    assert_eq!(plugin.adapter.cli_command, "wired_cli.sh");
    assert_eq!(plugin.adapter.profile, "test_profile");
}

#[test]
fn test_make_plugin_with_store_returns_existing_outbound_dir_matching_store() {
    let tmp = TempDir::new().unwrap();
    let (plugin, outbound_dir) = make_plugin_with_store("unused-cli", &tmp);

    assert_eq!(
        outbound_dir,
        plugin.adapter.media_store.outbound_dir(),
        "returned dir must match the plugin's media store outbound dir"
    );
    assert!(
        outbound_dir.exists(),
        "outbound dir must exist after assembly"
    );
    assert!(
        outbound_dir.starts_with(tmp.path()),
        "outbound dir must live under the passed tmp"
    );
}

#[test]
fn test_make_plugin_with_store_with_missing_cli_does_not_spawn_or_panic() {
    let tmp = TempDir::new().unwrap();

    let (plugin, outbound_dir) = make_plugin_with_store("unused-cli", &tmp);

    assert!(
        !tmp.path().join("unused-cli").exists(),
        "no mock script may be written for the passed cli_command"
    );
    assert_eq!(plugin.adapter.cli_command, "unused-cli");
    assert!(outbound_dir.exists());
}

#[test]
fn test_make_plugin_with_store_is_idempotent_over_same_tmp() {
    let tmp = TempDir::new().unwrap();

    let (plugin_a, dir_a) = make_plugin_with_store("unused-cli", &tmp);
    let (plugin_b, dir_b) = make_plugin_with_store("unused-cli", &tmp);

    assert_eq!(dir_a, dir_b, "same tmp must yield the same outbound dir");
    assert_eq!(dir_a, plugin_a.adapter.media_store.outbound_dir());
    assert_eq!(dir_b, plugin_b.adapter.media_store.outbound_dir());
}

// ===========================================================================
// Layered store primitives
// ===========================================================================

/// Normal path: each bare-store call backs the store with an independent
/// temp root (no shared state across call sites).
#[test]
fn test_make_test_media_store_yields_distinct_roots_per_call() {
    let store_a = make_test_media_store();
    let store_b = make_test_media_store();

    assert_ne!(
        store_a.outbound_dir(),
        store_b.outbound_dir(),
        "each call must back its store with an independent temp root"
    );
}

/// Given-root primitive: the store is rooted under the externally supplied
/// root (created on demand) and the store is writable.
#[test]
fn test_make_test_media_store_at_creates_store_under_given_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("media_root");

    let store = make_test_media_store_at(&root);

    assert!(root.exists(), "store root must be created if missing");
    assert!(
        store.outbound_dir().starts_with(&root),
        "store dirs must live under the given root"
    );
    let probe = store.outbound_dir().join("probe.txt");
    std::fs::write(&probe, "ok").unwrap();
    assert_eq!(std::fs::read_to_string(&probe).unwrap(), "ok");
}

/// Boundary: while the caller retains tmp ownership, the store's paths stay
/// valid and the store is writable (the with-tmp form's contract).
#[test]
fn test_make_test_media_store_with_tmp_keeps_store_writable_while_tmp_held() {
    let (tmp, store) = make_test_media_store_with_tmp();

    assert!(
        store.outbound_dir().starts_with(tmp.path()),
        "store root must live under the returned tmp"
    );
    assert!(
        store.outbound_dir().exists(),
        "outbound dir must stay valid while tmp ownership is retained"
    );
    let probe = store.outbound_dir().join("probe.txt");
    std::fs::write(&probe, "ok").unwrap();
    assert_eq!(std::fs::read_to_string(&probe).unwrap(), "ok");
}

// ===========================================================================
// Layered adapter assembly
// ===========================================================================

/// Normal path: the public adapter-with-store form behaves exactly like
/// `make_adapter` — same profile convention, cli override applied, store
/// rooted under the caller's tmp.
#[test]
fn test_make_adapter_with_store_matches_make_adapter_behavior() {
    let tmp = TempDir::new().unwrap();
    let adapter = make_adapter_with_store("my_lark_cli", &tmp);

    assert_eq!(
        adapter.cli_command, "my_lark_cli",
        "cli override must apply"
    );
    assert_eq!(
        adapter.profile, "test_profile",
        "shared assembly must keep the test_profile convention"
    );
    assert!(
        adapter.media_store.outbound_dir().starts_with(tmp.path()),
        "media store must be rooted under the passed tmp"
    );
    assert!(
        adapter.media_store.outbound_dir().exists(),
        "outbound dir must stay valid while tmp ownership is retained"
    );
}

/// Parameterized path: a custom profile is applied verbatim while the
/// default cli command stays untouched (plain `FeishuAdapter::new` parity).
#[test]
fn test_make_adapter_with_profile_applies_custom_profile_and_keeps_default_cli() {
    let adapter = make_adapter_with_profile("my_lark_profile");

    assert_eq!(
        adapter.profile, "my_lark_profile",
        "custom profile must be applied verbatim"
    );
    assert_eq!(
        adapter.cli_command, "lark-cli",
        "profile primitive must keep the default cli command"
    );
}

/// Boundary: repeated `make_adapter` calls are independent — each adapter
/// is backed by its own (self-managed, already-reclaimed) temp root, so
/// call sites never share media state or bleed cli overrides into each
/// other. Per the helper contract the store paths are never re-materialized
/// on disk after the call, so independence is asserted at path level only.
#[test]
fn test_make_adapter_calls_are_independent_per_invocation() {
    let a = make_adapter("cli_a.sh");
    let b = make_adapter("cli_b.sh");

    assert_ne!(
        a.media_store.outbound_dir(),
        b.media_store.outbound_dir(),
        "repeated calls must not share a media store root"
    );
    assert_eq!(
        a.cli_command, "cli_a.sh",
        "each call keeps its own override"
    );
    assert_eq!(
        b.cli_command, "cli_b.sh",
        "each call keeps its own override"
    );
}

// ===========================================================================
// Shared capability-error payload
// ===========================================================================

/// `CAPABILITY_ERROR_JSON` is the standard `error_json` argument of
/// `create_failing_mock_cli`, whose contract forbids single quotes (they
/// would break the generated script's shell quoting); the payload itself
/// must stay a well-formed capability-error JSON (code 230001).
#[test]
fn test_capability_error_json_is_shell_safe_and_wellformed() {
    assert!(
        !CAPABILITY_ERROR_JSON.contains('\''),
        "single quotes would break create_failing_mock_cli shell quoting"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(CAPABILITY_ERROR_JSON).expect("payload must be valid JSON");
    assert_eq!(
        parsed["code"],
        serde_json::json!(230001),
        "payload must keep the capability-error code"
    );
    assert_eq!(parsed["msg"], "capability error");
}
