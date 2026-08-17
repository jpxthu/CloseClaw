use super::active_searcher_llm::should_trigger_role;

#[test]
fn test_normal_role_triggers() {
    assert!(should_trigger_role("general"));
    assert!(should_trigger_role("assistant"));
    assert!(should_trigger_role("agent"));
}

#[test]
fn test_memory_miner_excluded() {
    assert!(!should_trigger_role("memory-miner"));
}

#[test]
fn test_dreaming_excluded() {
    assert!(!should_trigger_role("dreaming"));
}

#[test]
fn test_memory_miner_v2_excluded() {
    assert!(!should_trigger_role("memory-miner-v2"));
}

#[test]
fn test_memory_miner_custom_excluded() {
    assert!(!should_trigger_role("memory-miner-custom"));
}

#[test]
fn test_memory_not_excluded() {
    // "memory" is not a prefix match for "memory-miner"
    assert!(should_trigger_role("memory"));
}

#[test]
fn test_empty_string_not_excluded() {
    assert!(should_trigger_role(""));
}
