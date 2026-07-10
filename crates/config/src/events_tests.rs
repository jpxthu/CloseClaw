//! Tests for config change event types and broadcast channel.

use super::*;
use tokio::sync::broadcast::error::TryRecvError;

#[test]
fn test_broadcaster_new_has_zero_receivers() {
    let b = ConfigChangeBroadcaster::new();
    let _rx = b.subscribe();
    // After subscribing, the sender still works
    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: PathBuf::from("models.json"),
    });
}

#[test]
fn test_subscribe_receives_events() {
    let b = ConfigChangeBroadcaster::new();
    let mut rx = b.subscribe();

    let event = ConfigChangeEvent::Failed {
        section: ConfigSection::Channels,
        path: PathBuf::from("channels.json"),
        error: "bad json".into(),
    };
    b.send(event.clone());

    let received = rx.try_recv().unwrap();
    match received {
        ConfigChangeEvent::Failed {
            section,
            path,
            error,
        } => {
            assert_eq!(section, ConfigSection::Channels);
            assert_eq!(path, PathBuf::from("channels.json"));
            assert_eq!(error, "bad json");
        }
        other => panic!("unexpected event: {:?}", other),
    }
}

#[test]
fn test_multiple_subscribers_receive_events() {
    let b = ConfigChangeBroadcaster::new();
    let mut rx1 = b.subscribe();
    let mut rx2 = b.subscribe();

    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Gateway,
        path: PathBuf::from("gateway.json"),
    });

    assert!(rx1.try_recv().is_ok());
    assert!(rx2.try_recv().is_ok());
}

#[test]
fn test_send_without_subscribers_does_not_panic() {
    let b = ConfigChangeBroadcaster::new();
    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::System,
        path: PathBuf::from("system.json"),
    });
    // No panic, no error
}

#[test]
fn test_reloaded_event_clone() {
    let event = ConfigChangeEvent::Reloaded {
        section: ConfigSection::Plugins,
        path: PathBuf::from("plugins.json"),
    };
    let cloned = event.clone();
    match cloned {
        ConfigChangeEvent::Reloaded { section, path } => {
            assert_eq!(section, ConfigSection::Plugins);
            assert_eq!(path, PathBuf::from("plugins.json"));
        }
        _ => panic!("unexpected variant"),
    }
}

#[test]
fn test_default_capacity() {
    let b = ConfigChangeBroadcaster::default();
    let _rx = b.subscribe();
    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: PathBuf::from("models.json"),
    });
}

#[test]
fn test_broadcaster_with_custom_capacity() {
    let b = ConfigChangeBroadcaster::with_capacity(2);
    let mut rx = b.subscribe();
    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: PathBuf::from("models.json"),
    });
    let event = rx.try_recv().unwrap();
    match event {
        ConfigChangeEvent::Reloaded { section, path } => {
            assert_eq!(section, ConfigSection::Models);
            assert_eq!(path, PathBuf::from("models.json"));
        }
        _ => panic!("unexpected variant"),
    }
}

#[test]
fn test_subscribe_after_send_misses_old_events() {
    let b = ConfigChangeBroadcaster::new();
    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: PathBuf::from("models.json"),
    });
    // Subscribe AFTER the send — old event must not be replayed
    let mut rx = b.subscribe();
    match rx.try_recv() {
        Err(TryRecvError::Empty) => {} // expected
        other => panic!("expected Empty, got {:?}", other),
    }
}

#[test]
fn test_subscriber_dropped_then_send_does_not_panic() {
    let b = ConfigChangeBroadcaster::new();
    {
        let _rx = b.subscribe();
    } // _rx dropped here
    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Gateway,
        path: PathBuf::from("gateway.json"),
    });
    // No panic
}

#[test]
fn test_failed_event_clone() {
    let event = ConfigChangeEvent::Failed {
        section: ConfigSection::Channels,
        path: PathBuf::from("channels.json"),
        error: "timeout".into(),
    };
    let cloned = event.clone();
    match cloned {
        ConfigChangeEvent::Failed {
            section,
            path,
            error,
        } => {
            assert_eq!(section, ConfigSection::Channels);
            assert_eq!(path, PathBuf::from("channels.json"));
            assert_eq!(error, "timeout");
        }
        _ => panic!("unexpected variant"),
    }
}

#[test]
fn test_multiple_events_fifo_order() {
    let b = ConfigChangeBroadcaster::new();
    let mut rx = b.subscribe();

    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
        path: PathBuf::from("models.json"),
    });
    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Channels,
        path: PathBuf::from("channels.json"),
    });
    b.send(ConfigChangeEvent::Failed {
        section: ConfigSection::Gateway,
        path: PathBuf::from("gateway.json"),
        error: "bad".into(),
    });

    // Events must arrive in FIFO order
    let e1 = rx.try_recv().unwrap();
    assert!(matches!(
        e1,
        ConfigChangeEvent::Reloaded {
            section: ConfigSection::Models,
            ..
        }
    ));
    let e2 = rx.try_recv().unwrap();
    assert!(matches!(
        e2,
        ConfigChangeEvent::Reloaded {
            section: ConfigSection::Channels,
            ..
        }
    ));
    let e3 = rx.try_recv().unwrap();
    assert!(matches!(
        e3,
        ConfigChangeEvent::Failed {
            section: ConfigSection::Gateway,
            ..
        }
    ));
}
