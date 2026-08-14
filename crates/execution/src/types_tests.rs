use super::*;

#[test]
fn test_execution_config_default() {
    let config = ExecutionConfig::default();
    assert_eq!(config.mode, ExecutionMode::Inline);
    assert_eq!(config.verify_trigger, VerifyTrigger::NonTrivial);
    assert_eq!(config.step_selection, None);
}

#[test]
fn test_execution_config_default_matches_design_doc() {
    let config = ExecutionConfig::default();
    // Design doc: "inline 执行、per_step spawn、
    // 非平凡任务触发验证"
    assert_eq!(config.mode, ExecutionMode::Inline);
    assert_eq!(config.verify_trigger, VerifyTrigger::NonTrivial);
}

// --- ExecutionConfig.step_selection serde tests ---

/// step_selection None default roundtrip.
#[test]
fn test_execution_config_step_selection_none_default() {
    let config = ExecutionConfig::default();
    assert!(config.step_selection.is_none());
    let json = serde_json::to_string(&config).unwrap();
    let restored: ExecutionConfig = serde_json::from_str(&json).unwrap();
    assert!(restored.step_selection.is_none());
}

/// step_selection Some roundtrip.
#[test]
fn test_execution_config_step_selection_some_roundtrip() {
    let config = ExecutionConfig {
        step_selection: Some(vec![0, 1, 2]),
        ..ExecutionConfig::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    let restored: ExecutionConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.step_selection, Some(vec![0, 1, 2]));
}

/// step_selection empty vec roundtrip.
#[test]
fn test_execution_config_step_selection_empty_vec() {
    let config = ExecutionConfig {
        step_selection: Some(vec![]),
        ..ExecutionConfig::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    let restored: ExecutionConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.step_selection, Some(vec![]));
}

/// step_selection null in JSON → None.
#[test]
fn test_execution_config_step_selection_null_in_json() {
    let json = r#"{"mode": "inline", "verify_trigger": "non_trivial", "step_selection": null}"#;
    let config: ExecutionConfig = serde_json::from_str(json).unwrap();
    assert!(config.step_selection.is_none());
}

/// step_selection absent in JSON → None (serde default).
#[test]
fn test_execution_config_step_selection_absent_in_json() {
    let json = r#"{"mode": "inline", "verify_trigger": "non_trivial"}"#;
    let config: ExecutionConfig = serde_json::from_str(json).unwrap();
    assert!(config.step_selection.is_none());
}

/// step_selection with single element.
#[test]
fn test_execution_config_step_selection_single_element() {
    let config = ExecutionConfig {
        step_selection: Some(vec![3]),
        ..ExecutionConfig::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    let restored: ExecutionConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.step_selection, Some(vec![3]));
}

/// step_selection with large indices.
#[test]
fn test_execution_config_step_selection_large_indices() {
    let config = ExecutionConfig {
        step_selection: Some(vec![0, 100, 999]),
        ..ExecutionConfig::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    let restored: ExecutionConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.step_selection, Some(vec![0, 100, 999]));
}
