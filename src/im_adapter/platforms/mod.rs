//! Platform-specific IM plugins.
//!
//! Each sub-module implements the [`IMPlugin`](super::plugin::IMPlugin) trait
//! for a messaging platform.
//!
//! [`register_platform_plugins`] iterates over all platform modules and
//! delegates to each module's `register()` function so that new platforms
//! can be added by a single line change here.

pub mod feishu;

use std::sync::Arc;

/// Register all platform IM plugins with the Gateway.
///
/// Plugins that live under `platforms/` are discovered here.  Plugins that
/// do **not** belong in this directory (e.g. `TerminalPlugin`) are registered
/// explicitly elsewhere (design doc: "不在 `platforms/` 下的插件通过显式注册").
pub async fn register_platform_plugins(gateway: &Arc<crate::gateway::Gateway>, config_dir: &str) {
    feishu::register(gateway, config_dir).await;
}
