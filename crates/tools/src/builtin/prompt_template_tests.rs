//! Unit tests for `PromptTemplate::default_allowed_tools`.

use super::prompt_template::PromptTemplate;

// ---------------------------------------------------------------------------
// Explore template tests
// ---------------------------------------------------------------------------

#[test]
fn test_explore_default_tools_non_empty() {
    let tools = PromptTemplate::Explore
        .default_allowed_tools()
        .expect("Explore should have default tools");
    assert!(!tools.is_empty(), "Explore default tools must not be empty");
}

#[test]
fn test_explore_no_write_tools() {
    let tools = PromptTemplate::Explore
        .default_allowed_tools()
        .expect("Explore should have default tools");
    assert!(
        !tools.contains(&"Write"),
        "Explore must not include Write tool"
    );
    assert!(
        !tools.contains(&"Edit"),
        "Explore must not include Edit tool"
    );
    assert!(
        !tools.contains(&"GitCommit"),
        "Explore must not include GitCommit tool"
    );
    assert!(
        !tools.contains(&"plan_approval"),
        "Explore must not include plan_approval tool"
    );
}

#[test]
fn test_explore_includes_read_tools() {
    let tools = PromptTemplate::Explore
        .default_allowed_tools()
        .expect("Explore should have default tools");
    assert!(tools.contains(&"Read"), "Explore must include Read");
    assert!(tools.contains(&"Grep"), "Explore must include Grep");
    assert!(tools.contains(&"Ls"), "Explore must include Ls");
}

#[test]
fn test_explore_no_bash() {
    let tools = PromptTemplate::Explore
        .default_allowed_tools()
        .expect("Explore should have default tools");
    assert!(
        !tools.contains(&"Bash"),
        "Explore must not include Bash (read-only research)"
    );
}

// ---------------------------------------------------------------------------
// Plan template tests
// ---------------------------------------------------------------------------

#[test]
fn test_plan_default_tools_non_empty() {
    let tools = PromptTemplate::Plan
        .default_allowed_tools()
        .expect("Plan should have default tools");
    assert!(!tools.is_empty(), "Plan default tools must not be empty");
}

#[test]
fn test_plan_no_write_tools() {
    let tools = PromptTemplate::Plan
        .default_allowed_tools()
        .expect("Plan should have default tools");
    assert!(
        !tools.contains(&"Write"),
        "Plan must not include Write tool"
    );
    assert!(!tools.contains(&"Edit"), "Plan must not include Edit tool");
    assert!(
        !tools.contains(&"GitCommit"),
        "Plan must not include GitCommit tool"
    );
    assert!(
        !tools.contains(&"plan_approval"),
        "Plan must not include plan_approval tool"
    );
}

#[test]
fn test_plan_no_bash() {
    let tools = PromptTemplate::Plan
        .default_allowed_tools()
        .expect("Plan should have default tools");
    assert!(
        !tools.contains(&"Bash"),
        "Plan must not include Bash (read-only architect)"
    );
}

#[test]
fn test_plan_matches_explore_tools() {
    let explore = PromptTemplate::Explore.default_allowed_tools().unwrap();
    let plan = PromptTemplate::Plan.default_allowed_tools().unwrap();
    assert_eq!(
        explore, plan,
        "Plan and Explore should share the same tool set"
    );
}

// ---------------------------------------------------------------------------
// Validation template tests
// ---------------------------------------------------------------------------

#[test]
fn test_validation_default_tools_non_empty() {
    let tools = PromptTemplate::Validation
        .default_allowed_tools()
        .expect("Validation should have default tools");
    assert!(
        !tools.is_empty(),
        "Validation default tools must not be empty"
    );
}

#[test]
fn test_validation_includes_bash() {
    let tools = PromptTemplate::Validation
        .default_allowed_tools()
        .expect("Validation should have default tools");
    assert!(
        tools.contains(&"Bash"),
        "Validation must include Bash for running test scripts"
    );
}

#[test]
fn test_validation_no_write_tools() {
    let tools = PromptTemplate::Validation
        .default_allowed_tools()
        .expect("Validation should have default tools");
    assert!(
        !tools.contains(&"Write"),
        "Validation must not include Write tool"
    );
    assert!(
        !tools.contains(&"Edit"),
        "Validation must not include Edit tool"
    );
}

// ---------------------------------------------------------------------------
// Executor template tests
// ---------------------------------------------------------------------------

#[test]
fn test_executor_returns_none() {
    assert!(
        PromptTemplate::Executor.default_allowed_tools().is_none(),
        "Executor should return None (full toolset, no override)"
    );
}

// ---------------------------------------------------------------------------
// Cross-template consistency tests
// ---------------------------------------------------------------------------

#[test]
fn test_read_only_templates_excludes_approval() {
    let templates = [
        PromptTemplate::Explore,
        PromptTemplate::Plan,
        PromptTemplate::Validation,
    ];
    for tpl in &templates {
        let tools = tpl
            .default_allowed_tools()
            .expect("template should have tools");
        assert!(
            !tools.contains(&"plan_approval"),
            "{:?} must not include plan_approval",
            tpl
        );
    }
}

#[test]
fn test_all_templates_have_consistent_read_tools() {
    let templates = [
        PromptTemplate::Explore,
        PromptTemplate::Plan,
        PromptTemplate::Validation,
    ];
    for tpl in &templates {
        let tools = tpl
            .default_allowed_tools()
            .expect("template should have tools");
        assert!(tools.contains(&"Read"), "{:?} must include Read", tpl);
        assert!(tools.contains(&"Grep"), "{:?} must include Grep", tpl);
        assert!(tools.contains(&"Ls"), "{:?} must include Ls", tpl);
    }
}
