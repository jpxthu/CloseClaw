//! Process lifecycle management.
//!
//! Provides PID file read/write and signal-based process termination.
//! Uses SIGTERM/SIGINT on Unix.

use std::path::{Path, PathBuf};

use anyhow::Context;

/// Options for spawning a daemon process.
///
/// Controls the working directory, environment variables, and stdio
/// handling for the child process.
#[derive(Debug, Clone)]
pub struct SpawnOptions {
    /// Optional working directory for the child process.
    pub working_dir: Option<PathBuf>,
    /// Optional environment variables as key-value pairs.
    pub env_vars: Vec<(String, String)>,
    /// If `true`, stdin/stdout/stderr are redirected to `/dev/null`.
    /// Defaults to `true`.
    pub detach_stdio: bool,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            working_dir: None,
            env_vars: Vec::new(),
            detach_stdio: true,
        }
    }
}

use tracing::info;

/// Checks whether a process with the given PID is alive.
///
/// Uses `kill(pid, 0)` to probe. An `EPERM` error is treated
/// as "alive" (process exists but belongs to a different user).
pub fn is_process_alive(pid: u32) -> bool {
    // PIDs exceeding i32::MAX cannot exist on Unix (kernel pid_max is
    // typically 4194304). Casting to i32 would wrap to a negative value,
    // making kill(-1, 0) send to all processes — a false positive.
    let pid_i32 = match i32::try_from(pid) {
        Ok(p) => p,
        Err(_) => return false,
    };
    // SAFETY: kill with signal 0 is a standard POSIX existence check.
    // No signal is delivered; the kernel merely validates the PID.
    let ret = unsafe { libc::kill(pid_i32, 0) };
    if ret == 0 {
        return true;
    }
    // EPERM means the process exists but we lack permission to signal it.
    let err = std::io::Error::last_os_error();
    err.raw_os_error() == Some(libc::EPERM)
}

/// Checks a PID file for a stale or live daemon process.
///
/// * Returns `Some(pid)` if the PID file exists and the process is alive.
/// * Returns `Ok(None)` if the PID file does not exist.
/// * If the PID file exists but the process is dead (stale), the file is
///   deleted and `Ok(None)` is returned.
/// * Returns `Err` if the PID file cannot be read or removed.
pub fn check_stale_pid(pid_file: &Path) -> anyhow::Result<Option<u32>> {
    match read_pid_file(pid_file) {
        None => Ok(None),
        Some(pid) => {
            if is_process_alive(pid) {
                Ok(Some(pid))
            } else {
                // Stale PID file — remove it so the caller can start fresh.
                std::fs::remove_file(pid_file)?;
                Ok(None)
            }
        }
    }
}

/// Returns the PID file path: `{config_dir}/daemon.pid`.
pub fn pid_file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("daemon.pid")
}

/// Writes the given PID to the specified file, creating or overwriting it.
pub fn write_pid_file(path: &Path, pid: u32) -> anyhow::Result<()> {
    std::fs::write(path, pid.to_string())?;
    Ok(())
}

/// Reads a PID from the specified file.
///
/// Returns `None` if the file does not exist or cannot be parsed.
pub fn read_pid_file(path: &Path) -> Option<u32> {
    let content = std::fs::read_to_string(path).ok()?;
    content.trim().parse::<u32>().ok()
}

// ── stop_daemon ───────────────────────────────────────────────────

/// Result of a [`stop_daemon`] operation.
#[derive(Debug, PartialEq, Eq)]
pub enum StopOutcome {
    /// Daemon was stopped. Contains the PID that was terminated.
    Stopped(u32),
    /// Daemon was not running (PID file missing or stale; file cleaned up).
    NotRunning,
}

/// Deletes the PID file, ignoring "not found" errors.
fn cleanup_pid_file(pid_file: &Path) {
    let _ = std::fs::remove_file(pid_file);
}

/// Complete daemon stop sequence: read PID → signal → wait → cleanup.
///
/// Reads the daemon PID from `pid_file`, sends a termination signal,
/// waits for the process to exit within `timeout`, and removes the
/// PID file on success. If the PID file is missing or stale, returns
/// [`StopOutcome::NotRunning`] after cleaning up.
///
/// If the signal cannot be sent because the process has already exited
/// (ESRCH), the PID file is cleaned up and [`StopOutcome::NotRunning`]
/// is returned — no PID file is ever left behind.
///
/// "Zombie process" risk is borne by the daemon's real parent (init);
/// this module uses polling (`is_process_alive`) to confirm exit.
///
/// # Errors
///
/// Returns `Err` if the signal cannot be sent (for reasons other than
/// ESRCH) or the process does not exit within `timeout`.
pub fn stop_daemon(
    pid_file: &Path,
    force: bool,
    timeout: std::time::Duration,
) -> anyhow::Result<StopOutcome> {
    let pid = match read_pid_file(pid_file) {
        Some(pid) => pid,
        None => return Ok(StopOutcome::NotRunning),
    };
    if !is_process_alive(pid) {
        cleanup_pid_file(pid_file);
        return Ok(StopOutcome::NotRunning);
    }
    if let Err(e) = send_signal(pid, force) {
        // Exit race: process may have exited between is_process_alive
        // and send_signal. Check again; if gone, clean up and return.
        if !is_process_alive(pid) {
            cleanup_pid_file(pid_file);
            return Ok(StopOutcome::NotRunning);
        }
        return Err(e);
    }
    wait_for_exit(pid, timeout)?;
    cleanup_pid_file(pid_file);
    Ok(StopOutcome::Stopped(pid))
}

/// Waits for a process to exit, polling at 100ms intervals.
///
/// Uses [`is_process_alive`] as the probe primitive. Returns `Ok(())`
/// once the process is no longer alive. Returns `Err` if the process
/// is still alive after `timeout` elapses.
///
/// This is a synchronous blocking function (uses `std::thread::sleep`)
/// intended for CLI local operations only.
///
/// # Arguments
///
/// * `pid` - The process ID to wait for.
/// * `timeout` - Maximum duration to wait before returning an error.
///
/// # Errors
///
/// Returns an error if the process is still alive after the timeout.
pub fn wait_for_exit(pid: u32, timeout: std::time::Duration) -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_millis(100);
    loop {
        if !is_process_alive(pid) {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            anyhow::bail!(
                "Process {pid} did not exit within {}ms",
                timeout.as_millis()
            );
        }
        std::thread::sleep(poll_interval);
    }
}

/// Sends a termination signal to the process identified by `pid`.
///
/// Sends SIGTERM by default or SIGINT when `force` is true.
pub fn send_signal(pid: u32, force: bool) -> anyhow::Result<()> {
    let signal = if force { libc::SIGINT } else { libc::SIGTERM };
    let pid_i32 = i32::try_from(pid).context("PID exceeds i32::MAX")?;
    // SAFETY: kill with a valid signal is a standard POSIX operation.
    // pid_i32 is validated by i32::try_from above.
    let ret = unsafe { libc::kill(pid_i32, signal) };
    if ret != 0 {
        anyhow::bail!(
            "Failed to send signal to process {pid}: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

/// Spawns a daemon process, writes its PID file, and returns a child handle.
///
/// The daemon is started by executing the given command with the provided
/// arguments. After successful spawn, the child PID is written to
/// `{config_dir}/daemon.pid` using [`write_pid_file`].
///
/// # Arguments
///
/// * `command` - The program to execute (e.g. `"/usr/bin/my-daemon"`).
/// * `args` - Arguments to pass to the program.
/// * `config_dir` - Directory where `daemon.pid` will be written.
/// * `options` - Additional spawn configuration ([`SpawnOptions`]).
///
/// # Errors
///
/// Returns an error if the process cannot be spawned or if the PID file
/// cannot be written.
pub fn spawn_daemon(
    command: &str,
    args: &[&str],
    config_dir: &Path,
    options: &SpawnOptions,
) -> anyhow::Result<std::process::Child> {
    let mut cmd = std::process::Command::new(command);
    cmd.args(args);

    if let Some(ref dir) = options.working_dir {
        cmd.current_dir(dir);
    }

    for (key, value) in &options.env_vars {
        cmd.env(key, value);
    }

    if options.detach_stdio {
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }

    let child = cmd.spawn()?;
    let pid = child.id();

    let path = pid_file_path(config_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_pid_file(&path, pid)?;
    info!(pid, "Spawned daemon process");

    Ok(child)
}

/// Blocks until a shutdown signal is received.
///
/// Listens for both SIGINT (Ctrl+C) and SIGTERM.
pub async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = sigint.recv() => {
            info!("Received Ctrl+C, initiating shutdown...");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM, initiating graceful shutdown...");
        }
    }
    Ok(())
}
