//! Memory injection slot for active-searcher results.
//!
//! Each [`ConversationSession`](super::ConversationSession) holds an
//! `Arc<Mutex<Option<MemoryInjection>>>` so that the async active-searcher
//! task can write results while the session owner reads / consumes them
//! without a data race.

use std::collections::HashSet;

/// Where the memory-injection tool message should be placed
/// relative to the current user message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionPosition {
    /// Insert the tool message immediately after the current
    /// (most-recent) user message in the assembled list.
    AfterCurrent,
    /// Insert the tool message just before the next assistant
    /// message would appear (i.e. at the end of the list).
    BeforeNext,
}

/// A single memory-injection payload produced by the active-searcher.
///
/// Lifecycle per turn: **write** → **read/consume** → **clear**.
/// The slot is *not* persisted across process restarts.
#[derive(Debug, Clone)]
pub struct MemoryInjection {
    /// Summarised context text injected as a `role="tool"` message.
    pub content: String,
    /// Where to place the tool message in the assembled list.
    pub position_mode: InjectionPosition,
    /// Event IDs already injected this session — used for dedup
    /// so the same event is never injected twice.
    pub injected_event_ids: HashSet<i64>,
    /// Optional task ID identifying the source task that produced
    /// this injection. Used for session-level dedup so that the
    /// same task's results are injected at most once per session.
    pub task_id: Option<String>,
}

impl MemoryInjection {
    /// Create a new injection with the given content and position.
    /// The `injected_event_ids` set starts empty.
    pub fn new(content: String, position_mode: InjectionPosition) -> Self {
        Self {
            content,
            position_mode,
            injected_event_ids: HashSet::new(),
            task_id: None,
        }
    }

    /// Record that `event_id` has been injected so it won't appear
    /// again in future active-searcher turns.
    pub fn add_injected_event_id(&mut self, event_id: i64) {
        self.injected_event_ids.insert(event_id);
    }

    /// Returns `true` if `event_id` was already injected.
    pub fn is_event_injected(&self, event_id: i64) -> bool {
        self.injected_event_ids.contains(&event_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_injection_new_fields() {
        let inj = MemoryInjection::new("summary".into(), InjectionPosition::AfterCurrent);
        assert_eq!(inj.content, "summary");
        assert_eq!(inj.position_mode, InjectionPosition::AfterCurrent);
        assert!(inj.injected_event_ids.is_empty());
    }

    #[test]
    fn test_add_and_check_injected_event() {
        let mut inj = MemoryInjection::new("s".into(), InjectionPosition::BeforeNext);
        assert!(!inj.is_event_injected(42));
        inj.add_injected_event_id(42);
        assert!(inj.is_event_injected(42));
        assert!(!inj.is_event_injected(99));
    }

    #[test]
    fn test_add_injected_event_id_is_idempotent() {
        let mut inj = MemoryInjection::new("s".into(), InjectionPosition::AfterCurrent);
        inj.add_injected_event_id(1);
        inj.add_injected_event_id(1);
        inj.add_injected_event_id(1);
        assert_eq!(inj.injected_event_ids.len(), 1);
    }

    #[test]
    fn test_injection_clone_preserves_ids() {
        let mut inj = MemoryInjection::new("text".into(), InjectionPosition::AfterCurrent);
        inj.add_injected_event_id(10);
        inj.add_injected_event_id(20);
        let cloned = inj.clone();
        assert_eq!(cloned.injected_event_ids.len(), 2);
        assert!(cloned.is_event_injected(10));
        assert!(cloned.is_event_injected(20));
    }

    #[test]
    fn test_position_mode_equality() {
        assert_eq!(
            InjectionPosition::AfterCurrent,
            InjectionPosition::AfterCurrent
        );
        assert_eq!(InjectionPosition::BeforeNext, InjectionPosition::BeforeNext);
        assert_ne!(
            InjectionPosition::AfterCurrent,
            InjectionPosition::BeforeNext
        );
    }
}
