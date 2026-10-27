//! Daemon process spawning helpers.

use std::path::Path;
use std::process::Stdio;

use closeclaw_common::test_helpers::ensure_dir;
use tokio::process::{Child, Command};

/// Returns the path to the `closeclaw` daemon binary (not the test binary).
///
/// 用 Cargo 编译期注入的 `CARGO_BIN_EXE_closeclaw`，它会跟随 `CARGO_TARGET_DIR`；
/// 硬拼 `<manifest_dir>/target/debug` 在指定了自定义 target 目录的 job 里会找不到二进制。
pub(crate) fn closeclaw_binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_closeclaw"))
}

/// Spawn the closeclaw daemon in `--foreground` mode with HOME isolation.
///
/// Sets `HOME` to `config_root` so that `pid_file_path()` resolves to
/// `<config_root>/.closeclaw/daemon.pid`, isolating each test from the
/// global `~/.closeclaw/daemon.pid`.
///
/// Returns the `Child` handle. The caller is responsible for lifecycle
/// management (signal + wait). The daemon is spawned with `kill_on_drop`
/// to prevent residual processes on panic paths.
pub(crate) fn spawn_daemon(config_root: &Path) -> Child {
    // Ensure .closeclaw dir exists under temp HOME so PID file can be written
    ensure_dir(&config_root.join(".closeclaw")).expect("create .closeclaw dir");

    Command::new(closeclaw_binary())
        .args(["run", "--config-dir"])
        .arg(config_root.as_os_str())
        .arg("--foreground")
        .env("HOME", config_root.as_os_str())
        .current_dir(config_root.join("config"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn daemon")
}
