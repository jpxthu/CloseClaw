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
pub use fs::normalize_path;
pub use process::{send_signal, wait_for_shutdown_signal, write_pid_file};
pub use terminal::{current_uid, supports_ansi};

#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod fs_tests;
#[cfg(test)]
mod process_tests;
#[cfg(test)]
mod terminal_tests;
