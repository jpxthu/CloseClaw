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
