//! Configuration change event types and broadcast channel.
//!
//! Provides `ConfigChangeEvent` describing config section changes and
//! `ConfigChangeBroadcaster` for creating/subscribing to broadcast channels.

use std::path::PathBuf;

use crate::manager::ConfigSection;
use tokio::sync::broadcast;

/// Default capacity for the config change broadcast channel.
const DEFAULT_CHANNEL_CAPACITY: usize = 64;

/// A config change event describing which section changed and the outcome.
#[derive(Debug, Clone)]
pub enum ConfigChangeEvent {
    /// Config section was successfully reloaded.
    Reloaded {
        section: ConfigSection,
        path: PathBuf,
    },
    /// Config section reload failed (parse or validation error).
    Failed {
        section: ConfigSection,
        path: PathBuf,
        error: String,
    },
}

/// Broadcast sender for config change events.
///
/// Cheaply cloneable; multiple receivers can subscribe independently.
#[derive(Debug, Clone)]
pub struct ConfigChangeBroadcaster {
    tx: broadcast::Sender<ConfigChangeEvent>,
}

impl ConfigChangeBroadcaster {
    /// Create a new broadcaster with default channel capacity.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(DEFAULT_CHANNEL_CAPACITY);
        Self { tx }
    }

    /// Create a new broadcaster with a custom channel capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Subscribe to config change events.
    ///
    /// Returns a receiver that will receive all future events published
    /// after this subscription was created. Missed events (published
    /// before subscription) are not replayed.
    pub fn subscribe(&self) -> broadcast::Receiver<ConfigChangeEvent> {
        self.tx.subscribe()
    }

    /// Publish a config change event to all active subscribers.
    ///
    /// If there are no active subscribers the event is silently dropped.
    pub fn send(&self, event: ConfigChangeEvent) {
        // Ignore SendError: no active subscribers is not an error.
        let _ = self.tx.send(event);
    }
}

impl Default for ConfigChangeBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
