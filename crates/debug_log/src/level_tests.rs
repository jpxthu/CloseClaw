use super::*;

#[test]
fn test_ordering_trace_lt_debug() {
    assert!(LogLevel::Trace < LogLevel::Debug);
}

#[test]
fn test_ordering_debug_lt_info() {
    assert!(LogLevel::Debug < LogLevel::Info);
}

#[test]
fn test_ordering_info_lt_warn() {
    assert!(LogLevel::Info < LogLevel::Warn);
}

#[test]
fn test_ordering_warn_lt_error() {
    assert!(LogLevel::Warn < LogLevel::Error);
}

#[test]
fn test_ordering_full_chain() {
    let levels = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ];
    for w in levels.windows(2) {
        assert!(w[0] < w[1], "{:?} should be less than {:?}", w[0], w[1]);
    }
}

#[test]
fn test_equality() {
    assert_eq!(LogLevel::Info, LogLevel::Info);
    assert_ne!(LogLevel::Info, LogLevel::Warn);
}

#[test]
fn test_display_all_levels() {
    assert_eq!(LogLevel::Trace.to_string(), "trace");
    assert_eq!(LogLevel::Debug.to_string(), "debug");
    assert_eq!(LogLevel::Info.to_string(), "info");
    assert_eq!(LogLevel::Warn.to_string(), "warn");
    assert_eq!(LogLevel::Error.to_string(), "error");
}

#[test]
fn test_from_str_all_levels() {
    assert_eq!("trace".parse::<LogLevel>().unwrap(), LogLevel::Trace);
    assert_eq!("debug".parse::<LogLevel>().unwrap(), LogLevel::Debug);
    assert_eq!("info".parse::<LogLevel>().unwrap(), LogLevel::Info);
    assert_eq!("warn".parse::<LogLevel>().unwrap(), LogLevel::Warn);
    assert_eq!("error".parse::<LogLevel>().unwrap(), LogLevel::Error);
}

#[test]
fn test_from_str_case_insensitive() {
    assert_eq!("TRACE".parse::<LogLevel>().unwrap(), LogLevel::Trace);
    assert_eq!("Debug".parse::<LogLevel>().unwrap(), LogLevel::Debug);
    assert_eq!("INFO".parse::<LogLevel>().unwrap(), LogLevel::Info);
    assert_eq!("Warn".parse::<LogLevel>().unwrap(), LogLevel::Warn);
    assert_eq!("ERROR".parse::<LogLevel>().unwrap(), LogLevel::Error);
}

#[test]
fn test_from_str_invalid() {
    assert!("verbose".parse::<LogLevel>().is_err());
    assert!("".parse::<LogLevel>().is_err());
}

#[test]
fn test_display_from_str_roundtrip() {
    let levels = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ];
    for level in &levels {
        let s = level.to_string();
        let parsed: LogLevel = s.parse().unwrap();
        assert_eq!(*level, parsed, "roundtrip failed for {:?}", level);
    }
}

#[test]
fn test_serde_serialize() {
    let json = serde_json::to_string(&LogLevel::Info).unwrap();
    assert_eq!(json, r#""info""#);
}

#[test]
fn test_serde_deserialize() {
    let level: LogLevel = serde_json::from_str(r#""warn""#).unwrap();
    assert_eq!(level, LogLevel::Warn);
}

#[test]
fn test_serde_roundtrip() {
    let levels = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ];
    for level in &levels {
        let json = serde_json::to_string(level).unwrap();
        let parsed: LogLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(*level, parsed);
    }
}

#[test]
fn test_debug_trait() {
    assert_eq!(format!("{:?}", LogLevel::Info), "Info");
}
