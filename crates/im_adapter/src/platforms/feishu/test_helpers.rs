//! Shared mock-script helpers for feishu test modules.
//!
//! # 硬性约定（本目录所有测试适用）
//!
//! - 创建并实际 spawn mock 脚本的用例必须加 `#[serial_test::serial]`
//!   （ETXTBSY 根因）。
//! - mock 脚本必须落 `tempfile::TempDir`（系统临时目录），禁止仓库固定路径。

use super::{FeishuAdapter, FeishuPlugin};
use crate::media_store::MediaStore;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// Capability-error JSON emitted by the failing mock (stderr + stdout echo).
pub(crate) const CAPABILITY_ERROR_JSON: &str = r#"{"code":230001,"msg":"capability error"}"#;

/// Write a mock shell script atomically — temp-name write, handle
/// closed, rename into place, chmod 0755 — so the exec path never coexists
/// with an open write handle (avoids ETXTBSY on spawn).
pub(crate) fn write_mock_script(path: &std::path::Path, content: &str) {
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let script_tmp = std::path::PathBuf::from(tmp_name);
    std::fs::write(&script_tmp, content).unwrap(); // write & close before rename
    std::fs::rename(&script_tmp, path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, PermissionsExt::from_mode(0o755)).unwrap();
    }
}

/// Create a mock lark-cli script that echoes `response_json` to stdout and
/// exits 0 (script name unified as `mock_cli.sh` across all consumers).
///
/// Constraint: `response_json` must not contain single quotes — the generated
/// script embeds it in a single-quoted shell string, so a `'` inside would
/// break the quoting (same constraint as [`create_failing_mock_cli`]).
pub(crate) fn create_mock_cli(tmp: &TempDir, response_json: &str) -> String {
    let script_path = tmp.path().join("mock_cli.sh");
    let content = format!("#!/bin/bash\necho '{response_json}'\n");
    write_mock_script(&script_path, &content);
    script_path.to_str().unwrap().to_string()
}

/// Create a mock lark-cli script that always fails: emits `error_json` to
/// stderr, optionally echoes the same JSON to stdout, then exits with
/// `exit_code`.
///
/// Parameterized over the former per-file copies' differences: script
/// name / stderr JSON response / optional stdout echo / exit code.
///
/// Constraint: `error_json` must not contain single quotes — the generated
/// script embeds it in single-quoted shell strings, so a `'` inside would
/// break the quoting (and could inject shell syntax).
pub(crate) fn create_failing_mock_cli(
    tmp: &TempDir,
    script_name: &str,
    error_json: &str,
    should_echo_stdout: bool,
    exit_code: i32,
) -> String {
    let script_path = tmp.path().join(script_name);
    let mut content = format!("#!/bin/bash\necho '{error_json}' >&2\n");
    if should_echo_stdout {
        content.push_str(&format!("echo '{error_json}'\n"));
    }
    content.push_str(&format!("exit {exit_code}\n"));
    write_mock_script(&script_path, &content);
    script_path.to_str().unwrap().to_string()
}

/// Create a `MediaStore` rooted at an externally supplied `root`; the
/// `inbound/` and `outbound/` sub-directories are created under it.
pub(crate) fn make_test_media_store_at(root: &std::path::Path) -> Arc<MediaStore> {
    Arc::new(MediaStore::new(root.to_str().unwrap()).expect("media store"))
}

/// Create a `MediaStore` rooted in a fresh temp dir and hand back the
/// `TempDir` so callers that depend on path validity (e.g. outbound-copy
/// resolution against the store root) can keep the storage alive.
pub(crate) fn make_test_media_store_with_tmp() -> (TempDir, Arc<MediaStore>) {
    let tmp = TempDir::new().unwrap();
    let store = make_test_media_store_at(tmp.path());
    (tmp, store)
}

/// Create a `MediaStore` rooted in a self-managed temp dir. The temp dir is
/// removed as soon as this returns, so only use this when store paths are
/// never re-materialized on disk; for path-sensitive cases use
/// [`make_test_media_store_with_tmp`].
pub(crate) fn make_test_media_store() -> Arc<MediaStore> {
    make_test_media_store_with_tmp().1
}

/// Single assembly point: a `FeishuAdapter` wired to `cli_command` with a
/// media store rooted at `tmp` (profile `test_profile`).
pub(crate) fn make_adapter_with_store(cli_command: &str, tmp: &TempDir) -> FeishuAdapter {
    let media_store = make_test_media_store_at(tmp.path());
    let mut adapter = FeishuAdapter::new("test_profile".into(), media_store);
    adapter.cli_command = cli_command.to_string();
    adapter
}

/// Profile-parameterized assembly: a `FeishuAdapter` with the given profile
/// and the default cli command, backed by a self-managed temp-dir media
/// store — mirrors a plain `FeishuAdapter::new(profile, store)` call.
pub(crate) fn make_adapter_with_profile(profile: &str) -> FeishuAdapter {
    let media_store = make_test_media_store();
    FeishuAdapter::new(profile.into(), media_store)
}

/// Create a FeishuAdapter wired to `cli_command` with a self-built
/// temp-dir-backed media store (profile `test_profile`).
pub(crate) fn make_adapter(cli_command: &str) -> FeishuAdapter {
    let tmp = TempDir::new().unwrap();
    make_adapter_with_store(cli_command, &tmp)
}

/// Create a FeishuPlugin wired to `cli_command` with a media store rooted at
/// `tmp`; returns the store's outbound dir so tests can assert the
/// prepare-step outbound copy (proving the true upload path was taken).
/// Skip-path tests may ignore the return value as `_outbound_dir`;
/// discarding it is normal usage.
pub(crate) fn make_plugin_with_store(cli_command: &str, tmp: &TempDir) -> (FeishuPlugin, PathBuf) {
    let adapter = make_adapter_with_store(cli_command, tmp);
    let outbound_dir = adapter.media_store.outbound_dir().to_path_buf();
    let plugin = FeishuPlugin::new(Arc::new(adapter));
    (plugin, outbound_dir)
}

/// Create a FeishuPlugin wired to `cli_command` with a self-built
/// temp-dir-backed media store (delegates to [`make_adapter`]).
pub(crate) fn make_plugin(cli_command: &str) -> FeishuPlugin {
    FeishuPlugin::new(Arc::new(make_adapter(cli_command)))
}
