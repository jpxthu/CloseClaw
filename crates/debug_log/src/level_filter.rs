use crate::LogLevel;

/// Filters log events by minimum severity level.
///
/// Events below the configured level are discarded; events at or above are allowed through.
#[derive(Debug, Clone)]
pub struct LevelFilter {
    min_level: LogLevel,
}

impl LevelFilter {
    /// Create a new filter with the given minimum level.
    pub fn new(min_level: LogLevel) -> Self {
        Self { min_level }
    }

    /// Create a filter with the default level (`Debug` — intermediate state).
    pub fn default_filter() -> Self {
        Self::new(LogLevel::Debug)
    }

    /// Returns `true` if the given level meets or exceeds the minimum.
    pub fn should_log(&self, level: &LogLevel) -> bool {
        *level >= self.min_level
    }

    /// Get the current minimum level.
    pub fn min_level(&self) -> LogLevel {
        self.min_level
    }
}

impl Default for LevelFilter {
    fn default() -> Self {
        Self::default_filter()
    }
}
