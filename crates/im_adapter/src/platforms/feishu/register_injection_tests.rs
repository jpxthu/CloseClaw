//! Registration injection tests (composition-root port / injection dims).
//!
//! Covers the `register()` injection dimensions left after Steps 1.1–1.3:
//! - injected `feishu_profile` wins over the environment (resolution rule)
//! - `FEISHU_PROFILE` environment fallback when injection is `None`
//! - neither present → registration is skipped
//! - platform enabled → plugin registered through the `PluginRegistrar` port
//!
//! Env mutation is forbidden (docs/developer/STANDARDS.md §7),
//! so the fallback rule is exercised through the injected resolver seam and
//! the end-to-end cases run against a registrar that parks inside
//! `register_plugin` — the registration future is dropped at that point, so
//! `register()` never reaches the `ProcessManager` spawn (which would launch
//! the real `lark-cli`).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;

use super::plugin::resolve_feishu_profile;
use crate::media_store::MediaStore;
use crate::platforms::register_platform_plugins;
use crate::ports::test_doubles::{FakeEnqueuer, FakeRegistrar};
use crate::ports::GatewayHost;

/// Temp config dir with feishu explicitly enabled in `platforms.json`.
fn setup_enabled_config_dir() -> TempDir {
    let dir = TempDir::new().expect("tmp dir");
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).expect("config dir");
    std::fs::write(
        config.join("platforms.json"),
        r#"{"platforms":{"feishu":{"enabled":true}}}"#,
    )
    .expect("platforms.json");
    dir
}

/// Temp-dir media store so registration never falls back to the default
/// `~/.closeclaw/media` path.
fn make_media_store() -> Arc<MediaStore> {
    let tmp = TempDir::new().expect("media tmp");
    Arc::new(MediaStore::new(tmp.path().to_str().expect("utf-8")).expect("media store"))
}

/// Drive `register()` until it either completes (skip path) or parks inside
/// `FakeRegistrar::register_plugin`; returns whether a plugin was recorded.
/// Dropping the future at the park point guarantees no CLI spawn.
async fn drive_registration(fut: impl Future<Output = ()>, registrar: &FakeRegistrar) -> bool {
    let mut fut = std::pin::pin!(fut);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        tokio::select! {
            _ = &mut fut => {
                assert!(
                    registrar.registered().is_empty(),
                    "the skip path must return before registering a plugin"
                );
                return false;
            }
            _ = tokio::time::sleep(Duration::from_millis(5)) => {}
        }
        if !registrar.registered().is_empty() {
            return true;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "registration neither completed nor recorded a plugin"
        );
    }
}

// ===========================================================================
// Profile resolution rule (injected value wins, env value is the fallback)
// ===========================================================================

/// Injected profile is adopted without ever consulting the environment: the
/// env closure panics, proving `resolve_feishu_profile` short-circuits.
#[test]
fn test_resolve_profile_prefers_injected_over_env() {
    let resolved = resolve_feishu_profile(Some("injected_profile".to_string()), || {
        panic!("env fallback must not run when a profile is injected")
    });
    assert_eq!(resolved.as_deref(), Some("injected_profile"));
}

/// No injected profile → the environment value is the fallback source.
#[test]
fn test_resolve_profile_falls_back_to_env_when_not_injected() {
    let resolved = resolve_feishu_profile(None, || Some("env_profile".to_string()));
    assert_eq!(resolved.as_deref(), Some("env_profile"));
}

/// Neither present → no profile, which makes `register()` skip.
#[test]
fn test_resolve_profile_none_when_nothing_available() {
    let resolved: Option<String> = resolve_feishu_profile(None, || None);
    assert_eq!(resolved, None);
}

// ===========================================================================
// End-to-end registration orchestration (parking registrar, no CLI spawn)
// ===========================================================================

/// Platform enabled + injected profile → the plugin is registered through
/// the `PluginRegistrar` port (registration adoption of the injection).
#[tokio::test]
async fn test_register_enabled_registers_plugin_through_registrar() {
    let config_dir = setup_enabled_config_dir();
    let media_store = make_media_store();
    let registrar = FakeRegistrar::parking();
    let host = GatewayHost {
        enqueuer: FakeEnqueuer::new(),
        registrar: registrar.clone(),
        debug_log: None,
    };

    let config_dir_str = config_dir.path().to_str().expect("utf-8").to_string();
    let registered = drive_registration(
        register_platform_plugins(
            &host,
            &config_dir_str,
            Some(media_store),
            None,
            None,
            Some("injected_profile".to_string()),
        ),
        &registrar,
    )
    .await;

    assert!(
        registered,
        "enabled platform must register exactly one plugin"
    );
    let recorded = registrar.registered();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].platform(), "feishu");
}

/// Platform enabled but no profile anywhere → registration is skipped.
///
/// An ambient `FEISHU_PROFILE` would mask the skip path, and mutating the
/// process environment is forbidden (STANDARDS §7), so when the variable is
/// already set the whole case returns early and the registration future is
/// never driven; the skip assertion only runs when the environment is clean.
#[tokio::test]
async fn test_register_skips_when_no_profile_available() {
    if std::env::var("FEISHU_PROFILE").is_ok() {
        eprintln!("skip: ambient FEISHU_PROFILE set (env mutation forbidden by STANDARDS §7)");
        return;
    }
    let config_dir = setup_enabled_config_dir();
    let media_store = make_media_store();
    let registrar = FakeRegistrar::parking();
    let host = GatewayHost {
        enqueuer: FakeEnqueuer::new(),
        registrar: registrar.clone(),
        debug_log: None,
    };

    let config_dir_str = config_dir.path().to_str().expect("utf-8").to_string();
    let registered = drive_registration(
        register_platform_plugins(&host, &config_dir_str, Some(media_store), None, None, None),
        &registrar,
    )
    .await;

    assert!(
        !registered,
        "no injected or env profile must skip registration"
    );
}
