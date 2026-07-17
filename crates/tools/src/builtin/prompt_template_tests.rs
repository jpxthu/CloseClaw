//! Unit tests for `PromptTemplate` enum.

#![allow(clippy::unwrap_used)]

use std::str::FromStr;

use super::prompt_template::PromptTemplate;

// ---------------------------------------------------------------------------
// Normal path: from_str / to_string round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_from_str_validation_returns_ok() {
    let result = PromptTemplate::from_str("validation");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PromptTemplate::Validation);
}

#[test]
fn test_to_string_validation() {
    assert_eq!(PromptTemplate::Validation.to_string(), "validation");
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
// Backward-compat: old "verification" value rejected
// ---------------------------------------------------------------------------

#[test]
fn test_from_str_verification_rejected() {
    let result = PromptTemplate::from_str("verification");
    assert!(
        result.is_err(),
        "old value \"verification\" must no longer be accepted"
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
        ("validation", PromptTemplate::Validation),
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
        PromptTemplate::Validation,
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

#[test]
fn test_prefixes_all_non_empty() {
    let templates = [
        PromptTemplate::Explore,
        PromptTemplate::Validation,
        PromptTemplate::Plan,
        PromptTemplate::Executor,
    ];
    for tpl in &templates {
        assert!(!tpl.prefix().is_empty(), "{tpl:?} has empty prefix");
    }
}

// ---------------------------------------------------------------------------
// Validation prefix content correctness
// ---------------------------------------------------------------------------

#[test]
fn test_validation_prefix_mentions_verification_specialist() {
    let prefix = PromptTemplate::Validation.prefix();
    assert!(
        prefix.contains("verification specialist"),
        "Validation prefix must mention verification specialist"
    );
}

#[test]
fn test_validation_prefix_mentions_verdict() {
    let prefix = PromptTemplate::Validation.prefix();
    assert!(
        prefix.contains("VERDICT: PASS"),
        "Validation prefix must mention VERDICT: PASS"
    );
    assert!(
        prefix.contains("VERDICT: FAIL"),
        "Validation prefix must mention VERDICT: FAIL"
    );
}

#[test]
fn test_validation_prefix_mentions_pass_fail() {
    let prefix = PromptTemplate::Validation.prefix();
    assert!(
        prefix.contains("PASS"),
        "Validation prefix must mention PASS status"
    );
    assert!(
        prefix.contains("FAIL"),
        "Validation prefix must mention FAIL status"
    );
}

#[test]
fn test_validation_prefix_mentions_do_not_modify() {
    let prefix = PromptTemplate::Validation.prefix();
    assert!(
        prefix.contains("DO NOT MODIFY"),
        "Validation prefix must mention DO NOT MODIFY"
    );
}

// ---------------------------------------------------------------------------
// Explore prefix design doc marker tests
// ---------------------------------------------------------------------------

#[test]
fn test_explore_prefix_mentions_file_search_specialist() {
    let prefix = PromptTemplate::Explore.prefix();
    assert!(prefix.contains("file search specialist"));
}

#[test]
fn test_explore_prefix_read_only_mode() {
    let prefix = PromptTemplate::Explore.prefix();
    assert!(prefix.contains("READ-ONLY MODE"));
    assert!(prefix.contains("NO FILE MODIFICATIONS"));
}

#[test]
fn test_explore_prefix_communicate_final_report() {
    let prefix = PromptTemplate::Explore.prefix();
    assert!(prefix.contains("Communicate your final report directly"));
}

// ---------------------------------------------------------------------------
// Plan prefix design doc marker tests
// ---------------------------------------------------------------------------

#[test]
fn test_plan_prefix_mentions_software_architect() {
    let prefix = PromptTemplate::Plan.prefix();
    assert!(prefix.contains("software architect"));
}

#[test]
fn test_plan_prefix_critical_files() {
    let prefix = PromptTemplate::Plan.prefix();
    assert!(prefix.contains("Critical Files for Implementation"));
}

#[test]
fn test_plan_prefix_read_only() {
    let prefix = PromptTemplate::Plan.prefix();
    assert!(prefix.contains("READ-ONLY"));
}

// ---------------------------------------------------------------------------
// Executor prefix design doc marker tests
// ---------------------------------------------------------------------------

#[test]
fn test_executor_prefix_mentions_autonomous_execution() {
    let prefix = PromptTemplate::Executor.prefix();
    assert!(prefix.contains("autonomous execution mode"));
}

#[test]
fn test_executor_prefix_execute_immediately() {
    let prefix = PromptTemplate::Executor.prefix();
    assert!(prefix.contains("Execute immediately"));
}

#[test]
fn test_executor_prefix_avoid_data_exfiltration() {
    let prefix = PromptTemplate::Executor.prefix();
    assert!(prefix.contains("Avoid data exfiltration"));
}

#[test]
fn test_executor_prefix_all_six_rules() {
    let prefix = PromptTemplate::Executor.prefix();
    assert!(prefix.contains("1. Execute immediately"));
    assert!(prefix.contains("2. Minimize interruptions"));
    assert!(prefix.contains("3. Prefer action over planning"));
    assert!(prefix.contains("4. Expect course corrections"));
    assert!(prefix.contains("5. Do not take overly destructive actions"));
    assert!(prefix.contains("6. Avoid data exfiltration"));
}
