//! Plan Path clarity analysis.
//!
//! Determines whether a user request maps to the Standard path (clear
//! requirements) or the Interview path (ambiguous requirements) based
//! on heuristic signals in the user input text.

use closeclaw_common::PlanPath;

/// Analyze user input to determine the appropriate Plan Mode path.
///
/// Returns [`PlanPath::Standard`] when the request contains sufficient
/// specificity (file/module/interface references and quantifiable
/// acceptance criteria), otherwise [`PlanPath::Interview`].
pub fn analyze_plan_path(user_input: &str) -> PlanPath {
    let has_specificity = has_file_or_module_references(user_input);
    let has_quantifiable_criteria = has_acceptance_criteria(user_input);

    if has_specificity && has_quantifiable_criteria {
        PlanPath::Standard
    } else {
        PlanPath::Interview
    }
}

/// Detect file path or module references in the user input.
///
/// Signals: file paths with slashes, common source file extensions,
/// Rust module keywords, or `crate::` path prefixes.
fn has_file_or_module_references(input: &str) -> bool {
    let lower = input.to_lowercase();

    // File path patterns: src/, crates/, lib/, bin/, etc.
    if lower.contains("src/") || lower.contains("crates/") {
        return true;
    }

    // Source file extensions
    if lower.contains(".rs") || lower.contains(".py") || lower.contains(".ts") {
        return true;
    }

    // Rust module keywords
    if lower.contains("mod.rs") || lower.contains("lib.rs") || lower.contains("main.rs") {
        return true;
    }

    // Crate path prefix
    if lower.contains("crate::") {
        return true;
    }

    // Cargo workspace structure
    if lower.contains("cargo.toml") || lower.contains("workspace") {
        return true;
    }

    false
}

/// Detect quantifiable acceptance criteria in the user input.
///
/// Signals: explicit numeric targets, percentages, or prescriptive
/// modal verbs ("should", "must", "require", "ensure").
fn has_acceptance_criteria(input: &str) -> bool {
    let lower = input.to_lowercase();

    // Numeric thresholds
    if lower.contains("%") {
        return true;
    }

    // Prescriptive modals
    let modals = ["should", "must", "require", "ensure", "guarantee"];
    if modals.iter().any(|m| lower.contains(m)) {
        return true;
    }

    // Explicit numeric targets (e.g., "3 times", "100ms", "under 5s")
    if lower.contains("under ") || lower.contains("less than ") {
        return true;
    }

    false
}
