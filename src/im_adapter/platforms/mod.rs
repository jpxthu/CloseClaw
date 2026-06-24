//! Platform-specific IM plugins.
//!
//! Each sub-module implements the [`IMPlugin`](super::plugin::IMPlugin) trait
//! for a messaging platform.  Modules are auto-discovered at compile time by
//! `build.rs`, which scans this directory and writes `pub mod <name>;` lines
//! into `$OUT_DIR/platforms_gen.rs`.
//!
//! [`register_platform_plugins`] iterates over all [`PlatformEntry`] values
//! collected via [`inventory`] and delegates to each module's `register()`
//! function, so adding a new platform requires no changes here.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// Auto-generated module declarations (from build.rs).
include!(concat!(env!("OUT_DIR"), "/platforms_gen.rs"));

/// Registration function type for platform plugins.
///
/// Receives the Gateway handle and the configuration directory path.
pub type RegisterFn =
    fn(&Arc<crate::gateway::Gateway>, &str) -> Pin<Box<dyn Future<Output = ()> + Send>>;

/// A platform plugin entry discovered at compile time via [`inventory`].
///
/// Each platform module calls [`inventory::submit!`] with a `PlatformEntry`
/// to register itself.  [`register_platform_plugins`] then iterates over
/// all collected entries and invokes their `register` function.
pub struct PlatformEntry {
    /// Platform identifier (e.g. `"feishu"`).
    pub name: &'static str,
    /// Registration function.  Receives the Gateway handle and the
    /// configuration directory path.
    pub register: RegisterFn,
}

inventory::collect!(PlatformEntry);

/// Register all platform IM plugins with the Gateway.
///
/// Iterates over every [`PlatformEntry`] collected by [`inventory`] and
/// calls its `register` function.  Plugins that do **not** belong in
/// `platforms/` (e.g. `TerminalPlugin`) are registered explicitly elsewhere
/// (design doc: "不在 `platforms/` 下的插件通过显式注册").
pub async fn register_platform_plugins(gateway: &Arc<crate::gateway::Gateway>, config_dir: &str) {
    for entry in inventory::iter::<PlatformEntry> {
        (entry.register)(gateway, config_dir).await;
    }
}
