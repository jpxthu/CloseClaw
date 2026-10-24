use super::compute_default_selection;
use closeclaw_llm::model_info::ModelInfo;
use closeclaw_llm::ReasoningLevels;
use std::collections::HashSet;

#[cfg(test)]
fn test_model(id: &str) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        name: id.to_string(),
        context_window: 1000,
        max_tokens: 1000,
        default_temperature: None,
        reasoning: false,
        reasoning_levels: ReasoningLevels::None,
        input_types: vec![],
    }
}

#[test]
fn test_no_configured_models_returns_empty() {
    let models = vec![test_model("a"), test_model("b"), test_model("c")];
    let configured = HashSet::new();
    assert_eq!(compute_default_selection(&models, &configured), "");
}

#[test]
fn test_single_configured_model() {
    let models = vec![test_model("a"), test_model("b"), test_model("c")];
    let mut configured = HashSet::new();
    configured.insert("b".to_string());
    assert_eq!(compute_default_selection(&models, &configured), "2");
}

#[test]
fn test_multiple_configured_models_sorted() {
    let models = vec![
        test_model("a"),
        test_model("b"),
        test_model("c"),
        test_model("d"),
    ];
    let mut configured = HashSet::new();
    configured.insert("d".to_string());
    configured.insert("a".to_string());
    assert_eq!(compute_default_selection(&models, &configured), "1,4");
}

#[test]
fn test_all_models_configured() {
    let models = vec![test_model("a"), test_model("b"), test_model("c")];
    let configured: HashSet<String> = models.iter().map(|m| m.id.clone()).collect();
    assert_eq!(compute_default_selection(&models, &configured), "1,2,3");
}

#[test]
fn test_empty_model_list() {
    let models = vec![];
    let mut configured = HashSet::new();
    configured.insert("a".to_string());
    assert_eq!(compute_default_selection(&models, &configured), "");
}

#[test]
fn test_configured_ids_not_in_models_ignored() {
    let models = vec![test_model("a"), test_model("b")];
    let mut configured = HashSet::new();
    configured.insert("x".to_string());
    configured.insert("y".to_string());
    assert_eq!(compute_default_selection(&models, &configured), "");
}
