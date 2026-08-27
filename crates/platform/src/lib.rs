//! Platform abstraction layer.
//!
//! Provides unified interfaces for OS-specific operations including
//! terminal capability detection, process management, configuration
//! directory resolution, and file path normalization.

pub mod config;
pub mod fs;
pub mod process;
pub mod terminal;

pub use config::config_dir;
pub use fs::{
    check_executable, check_readable, check_writable, expand_home, normalize_path, set_executable,
    to_platform_path,
};
pub use process::{
    check_stale_pid, is_process_alive, pid_file_path, read_pid_file, send_signal, spawn_daemon,
    stop_daemon, wait_for_exit, wait_for_shutdown_signal, write_pid_file, SpawnOptions,
    StopOutcome,
};
pub use terminal::{
    current_uid, detect, is_terminal, read_line_raw, resolve_terminal_width, supports_ansi,
    write_raw, TerminalInfo,
};

#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod fs_tests;
#[cfg(test)]
mod process_tests;
#[cfg(test)]
mod terminal_tests;
