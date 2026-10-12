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

use std::fs;
use std::path::PathBuf;

use closeclaw_test_support::dep_boundary::{
    assert_workspace_dep_boundary, internal_dependency_names, DepBoundarySpec,
};

const ALLOWED_WORKSPACE_DEPS: [&str; 3] = [
    "closeclaw-common",
    "closeclaw-platform",
    "closeclaw-debug-log",
];

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

#[test]
fn test_regular_dependencies_stay_within_allowed_edge_table() {
    let spec = DepBoundarySpec::new("closeclaw-memory", manifest_path())
        .allowed_workspace_deps(&ALLOWED_WORKSPACE_DEPS);
    assert_workspace_dep_boundary(&spec);
}

#[test]
fn test_only_closeclaw_common_is_declared_as_internal_dep() {
    let manifest = fs::read_to_string(manifest_path()).expect("read this crate's Cargo.toml");
    let internal = internal_dependency_names(&manifest, &manifest_path())
        .expect("parse this crate's Cargo.toml");
    assert_eq!(
        internal,
        vec!["closeclaw-common"],
        "memory must depend on exactly one workspace crate: closeclaw-common"
    );
}
