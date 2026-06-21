//! Tests for config change event types and broadcast channel.

use super::*;

#[test]
fn test_broadcaster_new_has_zero_receivers() {
    let b = ConfigChangeBroadcaster::new();
    let _rx = b.subscribe();
    // After subscribing, the sender still works
    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::Models,
    });
}

#[test]
fn test_subscribe_receives_events() {
    let b = ConfigChangeBroadcaster::new();
    let mut rx = b.subscribe();

    let event = ConfigChangeEvent::Failed {
        section: ConfigSection::Channels,
        error: "bad json".into(),
    };
    b.send(event.clone());

    let received = rx.try_recv().unwrap();
    match received {
        ConfigChangeEvent::Failed { section, error } => {
            assert_eq!(section, ConfigSection::Channels);
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
    });

    assert!(rx1.try_recv().is_ok());
    assert!(rx2.try_recv().is_ok());
}

#[test]
fn test_send_without_subscribers_does_not_panic() {
    let b = ConfigChangeBroadcaster::new();
    b.send(ConfigChangeEvent::Reloaded {
        section: ConfigSection::System,
    });
    // No panic, no error
}

#[test]
fn test_reloaded_event_clone() {
    let event = ConfigChangeEvent::Reloaded {
        section: ConfigSection::Plugins,
    };
    let cloned = event.clone();
    match cloned {
        ConfigChangeEvent::Reloaded { section } => {
            assert_eq!(section, ConfigSection::Plugins);
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
    });
}
