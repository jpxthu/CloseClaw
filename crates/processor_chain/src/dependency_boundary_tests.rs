//! Dependency boundary guard tests.
//!
//! Enforces the dependency allowed-edge table (docs/design/STANDARDS.md):
//! this crate's regular dependency entries — key entries in [dependencies] /
//! [target.'…'.dependencies] and their `[dependencies.<name>]` /
//! `[target.'…'.dependencies.<name>]` sub-tables —
//! may only contain the workspace crates
//! {closeclaw-common, closeclaw-platform, closeclaw-debug-log}.
//! The manifest parsing and judgment live in the shared
//! `closeclaw-test-support` crate; its unit tests cover the parser behavior
//! cases.

use closeclaw_test_support::dep_boundary::{assert_workspace_dep_boundary, DepBoundarySpec};

const ALLOWED_WORKSPACE_DEPS: [&str; 3] = [
    "closeclaw-common",
    "closeclaw-platform",
    "closeclaw-debug-log",
];

/// Workspace crates that must stay dev-only: they may live in
/// [dev-dependencies] but never in regular dependency sections.
const DEV_ONLY_WORKSPACE_DEPS: [&str; 1] = ["closeclaw-im-adapter"];

/// Names banned from every regular dependency section, matched by name
/// regardless of declaration form (restores the original dev-only judgment
/// that also covered `tempfile`).
const FORBIDDEN_IN_REGULAR: [&str; 1] = ["tempfile"];

#[test]
fn test_regular_dependencies_stay_within_allowed_edge_table() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let spec = DepBoundarySpec::new("closeclaw-processor-chain", manifest)
        .allowed_workspace_deps(&ALLOWED_WORKSPACE_DEPS)
        .dev_only_workspace_deps(&DEV_ONLY_WORKSPACE_DEPS)
        .forbidden_in_regular(&FORBIDDEN_IN_REGULAR);
    assert_workspace_dep_boundary(&spec);
}
