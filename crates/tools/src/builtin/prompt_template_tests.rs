//! Unit tests for `PromptTemplate` enum.

#![allow(clippy::unwrap_used)]

use std::str::FromStr;

use super::prompt_template::PromptTemplate;

// ---------------------------------------------------------------------------
// Normal path: from_str / to_string round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_from_str_verification_returns_ok() {
    let result = PromptTemplate::from_str("verification");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PromptTemplate::Verification);
}

#[test]
fn test_to_string_verification() {
    assert_eq!(PromptTemplate::Verification.to_string(), "verification");
}

#[test]
fn test_from_str_explore_returns_ok() {
    let result = PromptTemplate::from_str("explore");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PromptTemplate::Explore);
}

#[test]
fn test_to_string_explore() {
    assert_eq!(PromptTemplate::Explore.to_string(), "explore");
}

#[test]
fn test_from_str_plan_returns_ok() {
    let result = PromptTemplate::from_str("plan");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PromptTemplate::Plan);
}

#[test]
fn test_to_string_plan() {
    assert_eq!(PromptTemplate::Plan.to_string(), "plan");
}

#[test]
fn test_from_str_executor_returns_ok() {
    let result = PromptTemplate::from_str("executor");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PromptTemplate::Executor);
}

#[test]
fn test_to_string_executor() {
    assert_eq!(PromptTemplate::Executor.to_string(), "executor");
}

// ---------------------------------------------------------------------------
// Backward-compat: old "validation" value rejected
// ---------------------------------------------------------------------------

#[test]
fn test_from_str_validation_rejected() {
    let result = PromptTemplate::from_str("validation");
    assert!(
        result.is_err(),
        "old value \"validation\" must no longer be accepted"
    );
}

#[test]
fn test_from_str_empty_string_rejected() {
    assert!(PromptTemplate::from_str("").is_err());
}

#[test]
fn test_from_str_unknown_value_rejected() {
    assert!(PromptTemplate::from_str("unknown").is_err());
    assert!(PromptTemplate::from_str("VALIDATION").is_err());
    assert!(PromptTemplate::from_str("Validation").is_err());
}

// ---------------------------------------------------------------------------
// Enum completeness: all 4 variants round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_all_variants_round_trip() {
    let variants = [
        ("explore", PromptTemplate::Explore),
        ("verification", PromptTemplate::Verification),
        ("plan", PromptTemplate::Plan),
        ("executor", PromptTemplate::Executor),
    ];
    for (s, expected) in variants {
        let parsed = PromptTemplate::from_str(s).unwrap();
        assert_eq!(parsed, expected, "from_str({s}) mismatch");
        assert_eq!(
            parsed.to_string(),
            s,
            "to_string of {:?} must produce {s}",
            expected
        );
    }
}

#[test]
fn test_all_variants_unique_string_values() {
    let all = [
        PromptTemplate::Explore,
        PromptTemplate::Verification,
        PromptTemplate::Plan,
        PromptTemplate::Executor,
    ];
    let strings: Vec<String> = all.iter().map(|v| v.to_string()).collect();
    let mut unique = strings.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        strings.len(),
        unique.len(),
        "all variants must produce distinct string values"
    );
}

// ---------------------------------------------------------------------------
// Tool whitelist: Verification gets read-only + Bash
// ---------------------------------------------------------------------------

#[test]
fn test_verification_default_tools_non_empty() {
    let tools = PromptTemplate::Verification
        .default_allowed_tools()
        .expect("Verification should have default tools");
    assert!(!tools.is_empty());
}

#[test]
fn test_verification_includes_bash() {
    let tools = PromptTemplate::Verification
        .default_allowed_tools()
        .expect("Verification should have default tools");
    assert!(
        tools.contains(&"Bash"),
        "Verification must include Bash for running test scripts"
    );
}

#[test]
fn test_verification_no_write_tools() {
    let tools = PromptTemplate::Verification
        .default_allowed_tools()
        .expect("Verification should have default tools");
    assert!(!tools.contains(&"Write"));
    assert!(!tools.contains(&"Edit"));
    assert!(!tools.contains(&"GitCommit"));
}

#[test]
fn test_verification_includes_read_tools() {
    let tools = PromptTemplate::Verification
        .default_allowed_tools()
        .expect("Verification should have default tools");
    assert!(tools.contains(&"Read"));
    assert!(tools.contains(&"Grep"));
    assert!(tools.contains(&"Ls"));
}

#[test]
fn test_verification_no_approval_tool() {
    let tools = PromptTemplate::Verification
        .default_allowed_tools()
        .expect("Verification should have default tools");
    assert!(!tools.contains(&"plan_approval"));
}

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
    assert!(!tools.contains(&"Write"));
    assert!(!tools.contains(&"Edit"));
    assert!(!tools.contains(&"GitCommit"));
    assert!(!tools.contains(&"plan_approval"));
}

#[test]
fn test_explore_includes_read_tools() {
    let tools = PromptTemplate::Explore
        .default_allowed_tools()
        .expect("Explore should have default tools");
    assert!(tools.contains(&"Read"));
    assert!(tools.contains(&"Grep"));
    assert!(tools.contains(&"Ls"));
}

#[test]
fn test_explore_no_bash() {
    let tools = PromptTemplate::Explore
        .default_allowed_tools()
        .expect("Explore should have default tools");
    assert!(!tools.contains(&"Bash"));
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
    assert!(!tools.contains(&"Write"));
    assert!(!tools.contains(&"Edit"));
    assert!(!tools.contains(&"GitCommit"));
    assert!(!tools.contains(&"plan_approval"));
}

#[test]
fn test_plan_no_bash() {
    let tools = PromptTemplate::Plan
        .default_allowed_tools()
        .expect("Plan should have default tools");
    assert!(!tools.contains(&"Bash"));
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
        PromptTemplate::Verification,
    ];
    for tpl in &templates {
        let tools = tpl
            .default_allowed_tools()
            .expect("template should have tools");
        assert!(!tools.contains(&"plan_approval"));
    }
}

#[test]
fn test_all_templates_have_consistent_read_tools() {
    let templates = [
        PromptTemplate::Explore,
        PromptTemplate::Plan,
        PromptTemplate::Verification,
    ];
    for tpl in &templates {
        let tools = tpl
            .default_allowed_tools()
            .expect("template should have tools");
        assert!(tools.contains(&"Read"));
        assert!(tools.contains(&"Grep"));
        assert!(tools.contains(&"Ls"));
    }
}

#[test]
fn test_prefixes_all_non_empty() {
    let templates = [
        PromptTemplate::Explore,
        PromptTemplate::Verification,
        PromptTemplate::Plan,
        PromptTemplate::Executor,
    ];
    for tpl in &templates {
        assert!(!tpl.prefix().is_empty(), "{tpl:?} has empty prefix");
    }
}
