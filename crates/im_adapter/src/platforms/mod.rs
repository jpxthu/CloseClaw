//! Platform-specific IM plugins.
//!
//! Each sub-module implements the [`IMPlugin`](closeclaw_common::IMPlugin) trait
//! for a messaging platform.  Modules are auto-discovered at compile time by
//! `build.rs`, which scans this directory and writes `pub mod <name>;` lines
//! into `$OUT_DIR/platforms_gen.rs`.

mod registry;

// Auto-generated module declarations (from build.rs).
include!(concat!(env!("OUT_DIR"), "/platforms_gen.rs"));

pub use registry::{register_platform_plugins, MediaConfigSnapshot, PlatformEntry, RegisterFn};
