use crate::plan_path::analyze_plan_path;
use closeclaw_common::PlanPath;

#[test]
fn test_analyze_standard_clear_with_paths_and_criteria() {
    let input =
        "Fix the bug in crates/system_prompt/src/sections.rs — the function should return None";
    assert_eq!(analyze_plan_path(input), PlanPath::Standard);
}

#[test]
fn test_analyze_standard_file_ext_and_modals() {
    let input = "Refactor lib.rs to handle errors properly, must not panic";
    assert_eq!(analyze_plan_path(input), PlanPath::Standard);
}

#[test]
fn test_analyze_interview_ambiguous() {
    let input = "Make it better";
    assert_eq!(analyze_plan_path(input), PlanPath::Interview);
}

#[test]
fn test_analyze_interview_no_quantifiable() {
    let input = "Fix the bug in src/main.rs";
    assert_eq!(analyze_plan_path(input), PlanPath::Interview);
}

#[test]
fn test_analyze_interview_no_file_reference() {
    let input = "This should be done in under 5 seconds";
    assert_eq!(analyze_plan_path(input), PlanPath::Interview);
}

#[test]
fn test_analyze_standard_crate_path_and_require() {
    let input = "Update crate::session to require auth on all endpoints";
    assert_eq!(analyze_plan_path(input), PlanPath::Standard);
}

#[test]
fn test_analyze_standard_percentage() {
    let input = "In crates/config/src/lib.rs, cache hit rate should be above 90%";
    assert_eq!(analyze_plan_path(input), PlanPath::Standard);
}

#[test]
fn test_analyze_interview_empty_input() {
    assert_eq!(analyze_plan_path(""), PlanPath::Interview);
}

#[test]
fn test_analyze_interview_vague_with_path() {
    let input = "Look into crates/tools/src/ and see what's going on";
    assert_eq!(analyze_plan_path(input), PlanPath::Interview);
}

#[test]
fn test_analyze_standard_cargo_toml() {
    let input = "Update Cargo.toml — the workspace should include the new crate";
    assert_eq!(analyze_plan_path(input), PlanPath::Standard);
}
