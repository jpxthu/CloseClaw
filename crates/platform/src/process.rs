//! Process lifecycle management.
//!
//! Provides PID file read/write and signal-based process termination.
//! Unix uses SIGTERM/SIGINT; Windows uses process termination API.

use std::path::{Path, PathBuf};

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
    /// If `true`, stdin/stdout/stderr are redirected to `/dev/null`
    /// (Unix) or `NUL` (Windows). Defaults to `true`.
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
/// On Unix, uses `kill(pid, 0)` to probe. An `EPERM` error is treated
/// as "alive" (process exists but belongs to a different user).
///
/// On Windows, queries `tasklist` and checks for the PID in the output.
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: kill with signal 0 is a standard POSIX existence check.
        // No signal is delivered; the kernel merely validates the PID.
        let ret = unsafe { libc::kill(pid as i32, 0) };
        if ret == 0 {
            return true;
        }
        // EPERM means the process exists but we lack permission to signal it.
        let err = std::io::Error::last_os_error();
        err.raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let output = match std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
        {
            Ok(o) => o,
            Err(_) => return false,
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        // tasklist /NH outputs a line with the PID if found, empty otherwise.
        stdout.contains(&pid.to_string())
    }
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

/// Returns the platform-specific PID file path.
///
/// On Unix: `{config_dir}/daemon.pid`
/// On Windows: `{config_dir}\daemon.pid`
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

/// Sends a termination signal to the process identified by `pid`.
///
/// On Unix, sends SIGTERM by default or SIGINT when `force` is true.
/// On Windows, uses `taskkill` without `/F` by default or with `/F`
/// when `force` is true.
pub fn send_signal(pid: u32, force: bool) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let signal = if force { libc::SIGINT } else { libc::SIGTERM };
        // SAFETY: kill with a valid signal is a standard POSIX operation.
        // The process ID is validated by the OS kernel.
        let ret = unsafe { libc::kill(pid as i32, signal) };
        if ret != 0 {
            anyhow::bail!(
                "Failed to send signal to process {pid}: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    #[cfg(not(unix))]
    {
        let mut args = vec!["/PID".to_string(), pid.to_string()];
        if force {
            args.push("/F".to_string());
        }
        let status = std::process::Command::new("taskkill")
            .args(&args)
            .status()?;
        if !status.success() {
            anyhow::bail!("Failed to terminate process {pid}");
        }
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

/// Blocks until a platform-appropriate shutdown signal is received.
///
/// On Unix, listens for both SIGINT (Ctrl+C) and SIGTERM.
/// On non-Unix platforms, listens for Ctrl+C only.
pub async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
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
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        info!("Received Ctrl+C, initiating shutdown...");
    }
    Ok(())
}
