//! In-memory event deduplicator for feishu platform events.
//!
//! Provides "at most once" semantics: duplicate `event_id`s are silently
//! dropped before any side-effects (media downloads, network calls, etc.).

use std::collections::{HashSet, VecDeque};

/// Default capacity for the event deduplicator (number of event IDs retained).
pub(crate) const DEFAULT_DEDUP_CAPACITY: usize = 4096;

/// In-memory event deduplicator -- a bounded FIFO set keyed by `event_id`.
///
/// When the set reaches capacity the oldest entry is evicted so new events
/// can always be accepted.
#[derive(Debug)]
pub(crate) struct EventDeduplicator {
    seen: HashSet<String>,
    order: VecDeque<String>,
    capacity: usize,
}

impl EventDeduplicator {
    /// Create a new deduplicator with the given maximum capacity.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            seen: HashSet::with_capacity(capacity.min(4096)),
            order: VecDeque::with_capacity(capacity.min(4096)),
            capacity,
        }
    }

    /// Return `true` if `event_id` has already been recorded.
    fn contains(&self, event_id: &str) -> bool {
        self.seen.contains(event_id)
    }

    /// Record `event_id` as seen, evicting the oldest entry if at capacity.
    fn insert(&mut self, event_id: String) {
        if self.seen.contains(&event_id) {
            return;
        }
        if self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        self.seen.insert(event_id.clone());
        self.order.push_back(event_id);
    }

    /// Check whether `event_id` is a duplicate and, if not, record it.
    ///
    /// Returns `true` when the event should be **dropped** (already seen).
    pub(crate) fn check_and_record(&mut self, event_id: &str) -> bool {
        if event_id.is_empty() {
            return false;
        }
        if self.contains(event_id) {
            return true;
        }
        self.insert(event_id.to_string());
        false
    }
}
