//! Tests for daemon config hot-reload module.

use super::*;
use closeclaw_config::events::{ConfigChangeBroadcaster, ConfigChangeEvent};
use closeclaw_config::manager::{ConfigManager, ConfigSection};
use closeclaw_gateway::{Gateway, GatewayConfig, SessionManager};
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;
use tempfile::TempDir;

/// Helper: create a ConfigManager backed by a temp directory.
fn make_config_manager(tmp: &TempDir) -> Arc<ConfigManager> {
    let config_dir = tmp.path().to_path_buf();
    Arc::new(ConfigManager::new(config_dir).expect("ConfigManager::new should succeed"))
}

/// Helper: create a SessionManager with defaults.
fn make_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &GatewayConfig::default(),
        None,
        None,
        ReasoningLevel::default(),
    ))
}

/// Helper: create a Gateway with defaults (for subscriber tests).
fn make_gateway() -> Arc<Gateway> {
    Arc::new(Gateway::new(
        GatewayConfig::default(),
        make_session_manager(),
    ))
}

// ---------------------------------------------------------------------------
// spawn_config_change_subscriber tests
// ---------------------------------------------------------------------------

/// Reloaded events should be received by the subscriber without panic.
#[tokio::test]
async fn test_subscriber_handles_reloaded_event() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();

    spawn_config_change_subscriber(Arc::clone(&config_mgr), session_mgr, make_gateway());

    // Give the spawned task a moment to start.
    tokio::task::yield_now().await;

    // Send a Reloaded event — subscriber should receive and call
    // notify_config_changed without panic.
    config_mgr.notify_change(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: "models.json".into(),
    });

    // Allow the spawned task to process the event.
    tokio::task::yield_now().await;
}

/// Failed events should be logged but NOT trigger a session notification.
#[tokio::test]
async fn test_subscriber_ignores_failed_event() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();

    spawn_config_change_subscriber(Arc::clone(&config_mgr), session_mgr, make_gateway());

    tokio::task::yield_now().await;

    // Send a Failed event — subscriber should log and skip notification.
    config_mgr.notify_change(ConfigChangeEvent::Failed {
        section: ConfigSection::Channels,
        path: "channels.json".into(),
        error: "test parse error".to_string(),
    });

    tokio::task::yield_now().await;
}

/// Multiple consecutive events are all processed without panic.
#[tokio::test]
async fn test_subscriber_handles_multiple_events() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let session_mgr = make_session_manager();

    spawn_config_change_subscriber(Arc::clone(&config_mgr), session_mgr, make_gateway());

    tokio::task::yield_now().await;

    let sections = [
        ConfigSection::Models,
        ConfigSection::Channels,
        ConfigSection::Gateway,
        ConfigSection::Plugins,
        ConfigSection::System,
    ];

    for section in sections {
        config_mgr.notify_change(ConfigChangeEvent::Reloaded {
            section,
            path: section.path(config_mgr.config_dir()),
        });
    }

    // Send a Failed event interleaved.
    config_mgr.notify_change(ConfigChangeEvent::Failed {
        section: ConfigSection::Models,
        path: "models.json".into(),
        error: "interleaved failure".to_string(),
    });

    // More Reloaded events after the failure.
    config_mgr.notify_change(ConfigChangeEvent::Reloaded {
        section: ConfigSection::System,
        path: "system.json".into(),
    });

    // Allow all events to be processed.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
}

/// When the broadcast channel is closed, the subscriber should exit cleanly
/// without panic.
#[tokio::test]
async fn test_subscriber_exits_on_channel_close() {
    let tmp = TempDir::new().unwrap();
    let config_mgr = make_config_manager(&tmp);
    let _session_mgr = make_session_manager();

    spawn_config_change_subscriber(Arc::clone(&config_mgr), _session_mgr, make_gateway());

    tokio::task::yield_now().await;

    // Drop the ConfigManager to close the broadcast channel.
    // The subscriber should receive `RecvError::Closed` and break.
    drop(config_mgr);

    // Allow the task to observe channel closure and exit.
    tokio::time::timeout(std::time::Duration::from_millis(200), async {
        loop {
            tokio::task::yield_now().await;
        }
    })
    .await
    .ok();
}

/// Broadcasting to a channel with no subscriber should not panic.
#[tokio::test]
async fn test_broadcast_no_subscribers_no_panic() {
    let broadcaster = ConfigChangeBroadcaster::new();

    // Sending with no receivers must not panic.
    broadcaster.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: "models.json".into(),
    });
    broadcaster.send(ConfigChangeEvent::Failed {
        section: ConfigSection::Channels,
        path: "channels.json".into(),
        error: "test".to_string(),
    });
}

/// Lagged events (subscriber too slow) should be handled gracefully.
#[tokio::test]
async fn test_subscriber_handles_lagged_events() {
    // Use a broadcaster with capacity 1 to easily cause lagging.
    let broadcaster = ConfigChangeBroadcaster::with_capacity(1);
    let mut rx = broadcaster.subscribe();

    // Send many events before the subscriber reads any — some will be lagged.
    for _ in 0..10 {
        broadcaster.send(ConfigChangeEvent::Reloaded {
            section: ConfigSection::Models,
            path: "models.json".into(),
        });
    }

    // Drop the sender so the channel closes after pending events are drained.
    // This prevents recv() from blocking indefinitely once the buffer is empty.
    drop(broadcaster);

    // The subscriber should handle RecvError::Lagged gracefully.
    // Read all pending events to confirm lag actually occurred.
    let mut got_lagged = false;
    loop {
        match rx.recv().await {
            Ok(ConfigChangeEvent::Reloaded { .. }) => {}
            Ok(ConfigChangeEvent::Failed { .. }) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                got_lagged = true;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
    // With capacity 1 and 10 sends, lag must have occurred.
    assert!(
        got_lagged,
        "expected at least one Lagged error with buffer capacity 1 and 10 sends"
    );
}

// ---------------------------------------------------------------------------
// Gap 2 — IM notification on config reload failure
// ---------------------------------------------------------------------------

/// parse_owner_target correctly parses a valid owner_display value.
#[test]
fn test_parse_owner_target_valid() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_path_buf();
    // Write system.json with owner_display
    let system_json = serde_json::json!({
        "commands": {
            "ownerDisplay": "feishu:oc_xxx123"
        }
    });
    std::fs::write(
        config_dir.join("system.json"),
        serde_json::to_string(&system_json).unwrap(),
    )
    .unwrap();
    let cm = ConfigManager::new(config_dir).unwrap();
    // Load only System section (others missing, but we only need System)
    let _ = cm.reload_section(ConfigSection::System, None);

    let result = parse_owner_target(&cm);
    assert_eq!(
        result,
        Some(("feishu".to_string(), "oc_xxx123".to_string()))
    );
}

/// parse_owner_target returns None when owner_display is not configured.
#[test]
fn test_parse_owner_target_not_configured() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_path_buf();
    // Write system.json without owner_display
    let system_json = serde_json::json!({ "version": "1.0" });
    std::fs::write(
        config_dir.join("system.json"),
        serde_json::to_string(&system_json).unwrap(),
    )
    .unwrap();
    let cm = ConfigManager::new(config_dir).unwrap();
    let _ = cm.reload_section(ConfigSection::System, None);

    let result = parse_owner_target(&cm);
    assert_eq!(result, None);
}

/// parse_owner_target returns None for invalid owner_display format.
#[test]
fn test_parse_owner_target_invalid_format() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_path_buf();
    // Missing colon separator
    let system_json = serde_json::json!({
        "commands": {
            "ownerDisplay": "no-colon-here"
        }
    });
    std::fs::write(
        config_dir.join("system.json"),
        serde_json::to_string(&system_json).unwrap(),
    )
    .unwrap();
    let cm = ConfigManager::new(config_dir).unwrap();
    let _ = cm.reload_section(ConfigSection::System, None);

    let result = parse_owner_target(&cm);
    assert_eq!(result, None);
}

/// parse_owner_target returns None when owner_display has empty parts.
#[test]
fn test_parse_owner_target_empty_parts() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_path_buf();
    let system_json = serde_json::json!({
        "commands": {
            "ownerDisplay": ":oc_xxx"
        }
    });
    std::fs::write(
        config_dir.join("system.json"),
        serde_json::to_string(&system_json).unwrap(),
    )
    .unwrap();
    let cm = ConfigManager::new(config_dir).unwrap();
    let _ = cm.reload_section(ConfigSection::System, None);

    let result = parse_owner_target(&cm);
    assert_eq!(result, None);
}

/// Subscriber handles Failed event when owner_display is configured.
/// Since no IM plugin is registered, send_outbound_simplified will fail
/// with UnknownChannel — the subscriber handles this gracefully.
#[tokio::test]
async fn test_subscriber_failed_event_with_owner_display() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_path_buf();
    // Write system.json with owner_display
    let system_json = serde_json::json!({
        "commands": {
            "ownerDisplay": "feishu:oc_test"
        }
    });
    std::fs::write(
        config_dir.join("system.json"),
        serde_json::to_string(&system_json).unwrap(),
    )
    .unwrap();
    let config_mgr = Arc::new(ConfigManager::new(config_dir).unwrap());
    let _ = config_mgr.reload_section(ConfigSection::System, None);

    let session_mgr = make_session_manager();
    spawn_config_change_subscriber(Arc::clone(&config_mgr), session_mgr, make_gateway());

    tokio::task::yield_now().await;

    // Send a Failed event — subscriber will try IM notification but
    // Gateway has no plugins registered, so it logs a warning.
    config_mgr.notify_change(ConfigChangeEvent::Failed {
        section: ConfigSection::Models,
        path: "models.json".into(),
        error: "test failure for IM notification".to_string(),
    });

    // Allow the spawned task to process the event without panic.
    tokio::task::yield_now().await;
    tokio::time::timeout(std::time::Duration::from_millis(200), async {
        loop {
            tokio::task::yield_now().await;
        }
    })
    .await
    .ok();
}
