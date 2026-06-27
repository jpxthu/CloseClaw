//! Turn counting logic for conversation sessions.

/// Counts the number of turns in a conversation session.
///
/// A "turn" corresponds to a user → assistant exchange.
/// Tool results also increment the turn count.
#[derive(Debug, Clone, Default)]
pub struct TurnCounter {
    count: u32,
}

impl TurnCounter {
    /// Creates a new turn counter starting at zero.
    pub fn new() -> Self {
        Self { count: 0 }
    }

    /// Increments the turn count by one.
    pub fn increment(&mut self) {
        self.count = self.count.saturating_add(1);
    }

    /// Returns the current turn count.
    pub fn count(&self) -> u32 {
        self.count
    }
}
