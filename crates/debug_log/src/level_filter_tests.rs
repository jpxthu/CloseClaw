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
