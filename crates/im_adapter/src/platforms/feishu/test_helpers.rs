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

/// Single assembly point: a `FeishuAdapter` wired to `cli_command` with a
/// media store rooted at `tmp` (profile `test_profile`).
fn adapter_with_store(cli_command: &str, tmp: &TempDir) -> FeishuAdapter {
    let media_store = Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"));
    let mut adapter = FeishuAdapter::new("test_profile".into(), media_store);
    adapter.cli_command = cli_command.to_string();
    adapter
}

/// Create a FeishuPlugin wired to `cli_command` with a media store rooted at
/// `tmp`; returns the store's outbound dir so tests can assert the
/// prepare-step outbound copy (proving the true upload path was taken).
/// Skip-path tests may ignore the return value as `_outbound_dir`;
/// discarding it is normal usage.
pub(crate) fn make_plugin_with_store(cli_command: &str, tmp: &TempDir) -> (FeishuPlugin, PathBuf) {
    let adapter = adapter_with_store(cli_command, tmp);
    let outbound_dir = adapter.media_store.outbound_dir().to_path_buf();
    let plugin = FeishuPlugin::new(Arc::new(adapter));
    (plugin, outbound_dir)
}

/// Create a FeishuAdapter wired to `cli_command` with a self-built
/// temp-dir-backed media store (profile `test_profile`).
pub(crate) fn make_adapter(cli_command: &str) -> FeishuAdapter {
    let tmp = TempDir::new().unwrap();
    adapter_with_store(cli_command, &tmp)
}

/// Create a FeishuPlugin wired to `cli_command` with a self-built
/// temp-dir-backed media store (delegates to [`make_adapter`]).
pub(crate) fn make_plugin(cli_command: &str) -> FeishuPlugin {
    FeishuPlugin::new(Arc::new(make_adapter(cli_command)))
}
