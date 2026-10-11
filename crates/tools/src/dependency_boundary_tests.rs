//! Dependency boundary guard tests.
//!
//! Enforces the dependency allowed-edge table (docs/design/STANDARDS.md):
//! this crate's regular dependency entries — key entries in [dependencies] /
//! [target.'…'.dependencies] and their `[dependencies.<name>]` /
//! `[target.'…'.dependencies.<name>]` sub-tables —
//! may only contain the workspace crates {closeclaw-common,
//! closeclaw-debug-log, closeclaw-platform}.
//! The manifest parsing and judgment live in the shared
//! `closeclaw-test-support` crate; its unit tests cover the parser behavior
//! cases.

use closeclaw_test_support::dep_boundary::{assert_workspace_dep_boundary, DepBoundarySpec};

const ALLOWED_WORKSPACE_DEPS: [&str; 3] = [
    "closeclaw-common",
    "closeclaw-debug-log",
    "closeclaw-platform",
];

/// Workspace crates that must stay dev-only: they may live in
/// [dev-dependencies] but never in regular dependency sections.
const DEV_ONLY_WORKSPACE_DEPS: [&str; 7] = [
    "closeclaw-tasks",
    "closeclaw-agent",
    "closeclaw-skills",
    "closeclaw-session",
    "closeclaw-config",
    "closeclaw-gateway",
    "closeclaw-permission",
];

#[test]
fn test_regular_dependencies_stay_within_allowed_edge_table() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let spec = DepBoundarySpec::new("closeclaw-tools", manifest)
        .allowed_workspace_deps(&ALLOWED_WORKSPACE_DEPS)
        .dev_only_workspace_deps(&DEV_ONLY_WORKSPACE_DEPS);
    assert_workspace_dep_boundary(&spec);
}
