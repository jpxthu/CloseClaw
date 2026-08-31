use super::generate_trace_id;

/// trace_id must have 3 parts separated by `_`.
#[test]
fn test_format_has_three_parts() {
    let id = generate_trace_id("tasks");
    let parts: Vec<&str> = id.split('_').collect();
    assert_eq!(parts.len(), 3, "trace_id must have 3 parts: {id}");
}

/// The first part must be the module name.
#[test]
fn test_module_name_preserved() {
    let id = generate_trace_id("daemon");
    assert!(
        id.starts_with("daemon_"),
        "trace_id must start with module name: {id}"
    );
}

/// The second part must be valid hex.
#[test]
fn test_timestamp_is_hex() {
    let id = generate_trace_id("tasks");
    let parts: Vec<&str> = id.split('_').collect();
    let ts = parts[1];
    assert!(!ts.is_empty(), "timestamp hex must not be empty: {id}");
    assert!(
        ts.chars().all(|c| c.is_ascii_hexdigit()),
        "timestamp must be hex digits: {id}"
    );
}

/// The third part must be a 32-char hex string (UUID without hyphens).
#[test]
fn test_random_hex_is_32_chars() {
    let id = generate_trace_id("tasks");
    let parts: Vec<&str> = id.split('_').collect();
    let random = parts[2];
    assert_eq!(random.len(), 32, "random hex must be 32 chars: {id}");
    assert!(
        random.chars().all(|c| c.is_ascii_hexdigit()),
        "random hex must be hex digits: {id}"
    );
}

/// The timestamp must be a reasonable recent value (within last 10s).
#[test]
fn test_timestamp_reasonable() {
    use chrono::Utc;

    let id = generate_trace_id("tasks");
    let parts: Vec<&str> = id.split('_').collect();
    let ts = u64::from_str_radix(parts[1], 16).expect("timestamp must be valid hex");
    let now_ms = Utc::now().timestamp_millis() as u64;
    assert!(
        ts <= now_ms && now_ms - ts < 10_000,
        "timestamp should be within last 10s: got {ts}, now {now_ms}"
    );
}

/// Two consecutive calls must produce different trace_ids.
#[test]
fn test_unique() {
    let id1 = generate_trace_id("tasks");
    let id2 = generate_trace_id("tasks");
    assert_ne!(id1, id2, "two consecutive trace_ids must differ");
}
