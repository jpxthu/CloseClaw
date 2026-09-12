//! Platform abstraction layer.
//!
//! Provides unified interfaces for OS-specific operations including
//! terminal capability detection, process management, configuration
//! directory resolution, and file path normalization.

pub mod config;
pub mod fs;
pub mod process;
pub mod terminal;

pub use config::root_dir;
pub use fs::{expand_env, expand_home, expand_path, normalize_path};
pub use process::{
    check_stale_pid, is_process_alive, pid_file_path, read_pid_file, send_signal, spawn_daemon,
    stop_daemon, subscribe_shutdown_signals, wait_for_exit, write_pid_file,
    ShutdownSignalSubscription, SpawnOptions, StopOutcome,
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
