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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_level_is_debug() {
        let filter = LevelFilter::default();
        assert_eq!(filter.min_level(), LogLevel::Debug);
    }

    #[test]
    fn test_default_filter_allows_debug_and_above() {
        let filter = LevelFilter::default();
        assert!(filter.should_log(&LogLevel::Debug));
        assert!(filter.should_log(&LogLevel::Info));
        assert!(filter.should_log(&LogLevel::Warn));
        assert!(filter.should_log(&LogLevel::Error));
    }

    #[test]
    fn test_default_filter_blocks_trace() {
        let filter = LevelFilter::default();
        assert!(!filter.should_log(&LogLevel::Trace));
    }

    #[test]
    fn test_custom_min_level_error() {
        let filter = LevelFilter::new(LogLevel::Error);
        assert!(!filter.should_log(&LogLevel::Trace));
        assert!(!filter.should_log(&LogLevel::Debug));
        assert!(!filter.should_log(&LogLevel::Info));
        assert!(!filter.should_log(&LogLevel::Warn));
        assert!(filter.should_log(&LogLevel::Error));
    }

    #[test]
    fn test_custom_min_level_info() {
        let filter = LevelFilter::new(LogLevel::Info);
        assert!(!filter.should_log(&LogLevel::Trace));
        assert!(!filter.should_log(&LogLevel::Debug));
        assert!(filter.should_log(&LogLevel::Info));
        assert!(filter.should_log(&LogLevel::Warn));
        assert!(filter.should_log(&LogLevel::Error));
    }

    #[test]
    fn test_exact_level_passes() {
        let filter = LevelFilter::new(LogLevel::Warn);
        assert!(filter.should_log(&LogLevel::Warn));
    }

    #[test]
    fn test_level_below_min_blocked() {
        let filter = LevelFilter::new(LogLevel::Warn);
        assert!(!filter.should_log(&LogLevel::Info));
        assert!(!filter.should_log(&LogLevel::Debug));
        assert!(!filter.should_log(&LogLevel::Trace));
    }

    #[test]
    fn test_level_above_min_passes() {
        let filter = LevelFilter::new(LogLevel::Debug);
        assert!(filter.should_log(&LogLevel::Info));
        assert!(filter.should_log(&LogLevel::Warn));
        assert!(filter.should_log(&LogLevel::Error));
    }
}
