//! Shared mock-script helpers for feishu test modules.
//!
//! # 硬性约定（本目录所有测试适用）
//!
//! - 创建并实际 spawn mock 脚本的用例必须加 `#[serial_test::serial]`
//!   （ETXTBSY 根因）。
//! - mock 脚本必须落 `tempfile::TempDir`（系统临时目录），禁止仓库固定路径。

use tempfile::TempDir;

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
pub(crate) fn create_failing_mock_cli(
    tmp: &TempDir,
    script_name: &str,
    error_json: &str,
    echo_stdout: bool,
    exit_code: i32,
) -> String {
    let script_path = tmp.path().join(script_name);
    let mut content = format!("#!/bin/bash\necho '{error_json}' >&2\n");
    if echo_stdout {
        content.push_str(&format!("echo '{error_json}'\n"));
    }
    content.push_str(&format!("exit {exit_code}\n"));
    write_mock_script(&script_path, &content);
    script_path.to_str().unwrap().to_string()
}
