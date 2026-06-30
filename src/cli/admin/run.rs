//! Run handler function for CLI admin.

use super::common::{json_output, RunOutput};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Default timeout for waiting on the admin socket (milliseconds).
const SOCKET_WAIT_TIMEOUT_MS: u64 = 30_000;

/// Interval between admin socket connection attempts (milliseconds).
const SOCKET_POLL_INTERVAL_MS: u64 = 200;

/// Returns the resolved config directory and the path to the PID file.
///
/// Separated from [`handle_run_foreground`] so that tests can verify
/// directory resolution without starting a real daemon.
pub fn prepare_run(config_dir: &str) -> Result<(PathBuf, PathBuf)> {
    let config_dir: PathBuf = if config_dir.is_empty() {
        closeclaw_platform::config::root_dir()?
    } else {
        PathBuf::from(config_dir)
    };
    std::fs::create_dir_all(&config_dir)?;

    let pid_file = closeclaw_platform::process::pid_file_path(&config_dir);

    Ok((config_dir, pid_file))
}

/// Foreground daemon runner — runs the daemon event loop in the current
/// process and writes the current PID to disk.  Called when
/// `--foreground` is passed.
pub async fn handle_run_foreground(config_dir: &str, json: bool) -> Result<()> {
    let (config_dir, pid_file) = prepare_run(config_dir)?;

    let pid = std::process::id();
    closeclaw_platform::process::write_pid_file(&pid_file, pid)?;

    crate::daemon::Daemon::start(config_dir.to_string_lossy().as_ref())
        .await?
        .run()
        .await?;

    if json {
        json_output(&RunOutput {
            pid,
            config_dir: config_dir.to_string_lossy().to_string(),
            started: true,
        });
        return Ok(());
    }

    println!("PID {} written to {}", pid, pid_file.display());
    println!("Daemon stopped.");
    Ok(())
}

/// Build the [`std::process::Command`] that spawns the daemon child process.
///
/// Extracted so that tests can verify argument construction without
/// actually spawning a process.
pub(crate) fn build_daemon_command(
    current_exe: &std::path::Path,
    config_dir: &std::path::Path,
) -> std::process::Command {
    let mut cmd = std::process::Command::new(current_exe);
    cmd.arg("run")
        .arg("--config-dir")
        .arg(config_dir)
        .arg("--foreground");

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    cmd
}

/// Start the daemon process.
///
/// When `foreground` is true, runs the daemon in the current process
/// (for debugging).  When false (default), spawns a child process to
/// run the daemon and waits for the admin socket to become available.
///
/// In JSON mode a [`RunOutput`] is printed instead of human-readable
/// text.
pub async fn handle_run(config_dir: String, json: bool, foreground: bool) -> Result<()> {
    if foreground {
        return handle_run_foreground(&config_dir, json).await;
    }

    // Background mode: spawn child process running the daemon.
    let (config_dir_path, pid_file) = prepare_run(&config_dir)?;

    let current_exe =
        std::env::current_exe().context("failed to resolve current executable path")?;

    let mut cmd = build_daemon_command(&current_exe, &config_dir_path);

    let _child = cmd.spawn().context("failed to spawn daemon process")?;

    // Wait for the admin socket to become available.
    let admin_socket = crate::admin::client::admin_socket_path(&config_dir_path);
    wait_for_socket(&admin_socket, SOCKET_WAIT_TIMEOUT_MS)?;

    // Read the PID that the child process wrote to the PID file.
    let pid = closeclaw_platform::process::read_pid_file(&pid_file)
        .context("failed to read daemon PID file after spawn")?;

    if json {
        json_output(&RunOutput {
            pid,
            config_dir: config_dir_path.to_string_lossy().to_string(),
            started: true,
        });
        return Ok(());
    }

    println!(
        "Daemon started (PID {}) — config: {}",
        pid,
        config_dir_path.display()
    );
    Ok(())
}

/// Poll the given Unix socket path until a connection succeeds or the
/// timeout expires.
pub(crate) fn wait_for_socket(path: &Path, timeout_ms: u64) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

    loop {
        // Attempt a synchronous connection to the socket.
        if try_connect(path).is_ok() {
            return Ok(());
        }

        if std::time::Instant::now() >= deadline {
            anyhow::bail!(
                "Timed out waiting for daemon admin socket at {}",
                path.display()
            );
        }

        std::thread::sleep(std::time::Duration::from_millis(SOCKET_POLL_INTERVAL_MS));
    }
}

/// Try connecting to the given Unix socket path once.
#[cfg(unix)]
pub(crate) fn try_connect(path: &Path) -> Result<()> {
    use std::os::unix::net::UnixStream;
    UnixStream::connect(path)
        .with_context(|| format!("socket connect failed: {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn try_connect(_path: &Path) -> Result<()> {
    anyhow::bail!("Unix sockets are not supported on this platform")
}
