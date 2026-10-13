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

#[cfg(test)]
mod tests {
    //! Convention (see module docs above): any test that creates and actually
    //! spawns a mock CLI script must be marked `#[serial_test::serial]`
    //! (ETXTBSY root cause); mock scripts must live in a `tempfile::TempDir`.

    use super::*;
    use serial_test::serial;
    use std::os::unix::fs::PermissionsExt;

    fn assert_executable(path: &std::path::Path) {
        let mode = std::fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "script must be executable (0755)");
    }

    #[test]
    fn write_mock_script_writes_content_to_existing_executable_path() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("normal.sh");
        let content = "#!/bin/bash\necho 'hello'\n";

        write_mock_script(&path, content);

        assert!(path.exists(), "returned target path must exist");
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, content, "file content must match input verbatim");
        assert_executable(&path);
    }

    #[test]
    fn write_mock_script_overwrites_existing_target_without_tmp_leftover() {
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
    fn failing_creator_places_executable_script_under_given_name() {
        let tmp = TempDir::new().unwrap();
        let cli = create_failing_mock_cli(&tmp, "failing_lark_cli.sh", r#"{"code":1}"#, false, 1);

        let path = std::path::PathBuf::from(&cli);
        assert_eq!(path, tmp.path().join("failing_lark_cli.sh"));
        assert!(path.exists());
        assert_executable(&path);
    }

    #[test]
    fn failing_creator_emits_error_json_on_stderr_only() {
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
            "echo_stdout=false must not add a stdout echo line"
        );
    }

    #[test]
    fn failing_creator_stdout_echo_follows_flag() {
        let tmp = TempDir::new().unwrap();
        let json = r#"{"code":2}"#;

        let loud = create_failing_mock_cli(&tmp, "echo_on.sh", json, true, 1);
        let loud_content = std::fs::read_to_string(&loud).unwrap();
        assert!(
            loud_content.contains(&format!("echo '{json}'\n")),
            "echo_stdout=true must echo the same JSON to stdout"
        );

        let quiet = create_failing_mock_cli(&tmp, "echo_off.sh", json, false, 1);
        let quiet_content = std::fs::read_to_string(&quiet).unwrap();
        assert_eq!(
            quiet_content.matches(&format!("echo '{json}'")).count(),
            1,
            "echo_stdout=false must emit the JSON exactly once (stderr only)"
        );
    }

    #[test]
    fn failing_creator_bakes_exit_code_into_script() {
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
    fn failing_creator_spawned_script_matches_spec() {
        let tmp = TempDir::new().unwrap();
        let json = r#"{"code":9,"msg":"boom"}"#;
        let cli = create_failing_mock_cli(&tmp, "spawn_spec.sh", json, true, 3);

        let output = std::process::Command::new(&cli).output().unwrap();
        assert_eq!(output.status.code(), Some(3), "exit code must propagate");
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), json);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), json);
    }
}
