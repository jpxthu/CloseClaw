//! Platform plugin registry: entry type and startup registration.
//!
//! [`register_platform_plugins`] iterates over all [`PlatformEntry`] values
//! collected via [`inventory`] and delegates to each module's `register()`
//! function, so adding a new platform requires no changes here.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use closeclaw_common::identity::IdentityResolver;

/// Read-only projection of the media configuration injected by the
/// composition root (daemon) at platform registration time.
///
/// `im_adapter` must not depend on the config crate, so the daemon
/// projects the config media data onto this data primitive
/// field-by-field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaConfigSnapshot {
    /// Directory path for media file storage (supports `~` expansion).
    pub storage_dir: PathBuf,
    /// Number of days to retain media files (`0` disables cleanup).
    pub retention_days: u64,
    /// Image content threshold in bytes.
    pub image_content_threshold_bytes: u64,
}

impl Default for MediaConfigSnapshot {
    /// Mirrors the config crate's media data defaults — that crate owns
    /// these constants; the composition root projects them and a
    /// daemon-side drift test pins the two defaults together.
    fn default() -> Self {
        Self {
            storage_dir: PathBuf::from("~/.closeclaw/media"),
            retention_days: 7,
            image_content_threshold_bytes: 1_048_576,
        }
    }
}

/// Registration function type for platform plugins.
///
/// Receives the Gateway handle, the configuration directory path,
/// an optional shared MediaStore, the injected media-config snapshot,
/// the injected identity resolver, and the injected platform profile.
pub type RegisterFn = fn(
    &Arc<closeclaw_gateway::Gateway>,
    &str,
    Option<std::sync::Arc<crate::media_store::MediaStore>>,
    Option<MediaConfigSnapshot>,
    Option<Arc<dyn IdentityResolver>>,
    Option<String>,
) -> Pin<Box<dyn Future<Output = ()> + Send>>;

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
pub async fn register_platform_plugins(
    gateway: &Arc<closeclaw_gateway::Gateway>,
    config_dir: &str,
    media_store: Option<std::sync::Arc<crate::media_store::MediaStore>>,
    media_config: Option<MediaConfigSnapshot>,
    identity_resolver: Option<Arc<dyn IdentityResolver>>,
    feishu_profile: Option<String>,
) {
    for entry in inventory::iter::<PlatformEntry> {
        (entry.register)(
            gateway,
            config_dir,
            media_store.clone(),
            media_config.clone(),
            identity_resolver.clone(),
            feishu_profile.clone(),
        )
        .await;
    }
}
