//! Cross-crate consistency tests for the `plans/` path predicate.
//!
//! Tools' [`is_plan_file_path`] and permission's `is_plans_path` are
//! deliberately isomorphic (see the doc comments on both). These tests
//! assert the two agree on the same path samples so drift between the
//! crates is caught by CI. Only same-value agreement is asserted here —
//! semantic correctness of each predicate is covered by the owning
//! crate's own tests (permission `engine_plan_mode_tests`, tools
//! write/edit plan-path tests).

use crate::builtin::plans_route::is_plan_file_path;
use closeclaw_permission::engine::engine_mode_filter::is_plans_path;

/// Path samples on which both predicates must agree.
const SAMPLES: &[&str] = &[
    // Prefix positives.
    "plans/",
    "plans/x.md",
    // Contains positives.
    "a/plans/b.md",
    "/abs/plans/x.md",
    "./plans/x.md",
    // Both branches true (starts_with AND contains).
    "plans/plans/x.md",
    // Negatives: no trailing slash, singular, prefix trap, tail without
    // slash, case-sensitive, empty, plain path.
    "plans",
    "plan/x.md",
    "some-plans-dir/x.md",
    "a/plans",
    "PLANS/x.md",
    "",
    "src/main.rs",
];

#[test]
fn test_plans_path_predicates_agree_on_all_samples() {
    for sample in SAMPLES {
        assert_eq!(
            is_plan_file_path(sample),
            is_plans_path(sample),
            "predicates diverge on {sample:?}"
        );
    }
}
