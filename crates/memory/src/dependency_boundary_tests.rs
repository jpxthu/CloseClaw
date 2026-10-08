//! Dependency boundary guard tests.
//!
//! Enforces the dependency allowed-edge table (docs/design/STANDARDS.md):
//! this crate's regular dependency entries — key entries in [dependencies] /
//! [target.'…'.dependencies] and their `[dependencies.<name>]` /
//! `[target.'…'.dependencies.<name>]` sub-tables —
//! may only contain the workspace crates
//! {closeclaw-common, closeclaw-platform, closeclaw-debug-log}.
//!
//! The manifest is parsed with the `toml` crate (dev-dependency): inline
//! tables, sub-tables, dotted keys, multi-line tables, and comments are
//! handled by the real TOML grammar instead of hand-rolled scanners.

use std::fs;
use std::path::PathBuf;

use toml::Value;

const ALLOWED_WORKSPACE_DEPS: [&str; 3] = [
    "closeclaw-common",
    "closeclaw-platform",
    "closeclaw-debug-log",
];

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

/// Regular dependency entries of a manifest as `(name, declaration)` pairs:
/// the `[dependencies]` section plus every `[target.'…'.dependencies]`
/// section. Dev, build, and target-dev sections stay out of scope; the
/// `[dependencies.<name>]` sub-table form arrives as the dependency's
/// declaration table, exactly like its inline-table form.
fn regular_dependencies(manifest: &str) -> Vec<(String, Value)> {
    let table: Value = toml::from_str(manifest).expect("manifest must be valid TOML");
    let mut dependencies: Vec<(String, Value)> = Vec::new();
    if let Some(section) = table.get("dependencies").and_then(Value::as_table) {
        dependencies.extend(
            section
                .iter()
                .map(|(name, declaration)| (name.to_string(), declaration.clone())),
        );
    }
    let targets = table.get("target").and_then(Value::as_table);
    for target in targets.into_iter().flat_map(toml::Table::values) {
        if let Some(section) = target.get("dependencies").and_then(Value::as_table) {
            dependencies.extend(
                section
                    .iter()
                    .map(|(name, declaration)| (name.to_string(), declaration.clone())),
            );
        }
    }
    dependencies
}

/// A declaration is workspace-internal only through TOML structure: a `path`
/// key, or `workspace = true` inheritance — recognized by key, never by
/// substring matching. String declarations are version-only crates.io deps.
fn is_workspace_internal_declaration(declaration: &Value) -> bool {
    declaration.as_table().is_some_and(|table| {
        table.contains_key("path") || table.get("workspace") == Some(&Value::Boolean(true))
    })
}

fn workspace_internal_violations<'a>(dependencies: &'a [(String, Value)]) -> Vec<&'a str> {
    let mut violations: Vec<&'a str> = dependencies
        .iter()
        .filter(|(_, declaration)| is_workspace_internal_declaration(declaration))
        .map(|(name, _)| name.as_str())
        .filter(|name| !ALLOWED_WORKSPACE_DEPS.contains(name))
        .collect();
    violations.sort_unstable();
    violations.dedup();
    violations
}

#[test]
fn test_regular_dependencies_stay_within_allowed_edge_table() {
    let manifest = fs::read_to_string(manifest_path()).expect("read this crate's Cargo.toml");
    let dependencies = regular_dependencies(&manifest);
    assert!(
        !dependencies.is_empty(),
        "parser must extract the [dependencies] entries"
    );
    let violations = workspace_internal_violations(&dependencies);
    assert!(
        violations.is_empty(),
        "workspace deps outside the allowed edge table: {violations:?}"
    );
}

#[test]
fn test_only_closeclaw_common_is_declared_as_internal_dep() {
    let manifest = fs::read_to_string(manifest_path()).expect("read this crate's Cargo.toml");
    let dependencies = regular_dependencies(&manifest);
    let internal: Vec<&str> = dependencies
        .iter()
        .filter(|(_, declaration)| is_workspace_internal_declaration(declaration))
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        internal,
        vec!["closeclaw-common"],
        "memory must depend on exactly one workspace crate: closeclaw-common"
    );
}

#[test]
fn test_out_of_bound_internal_dependency_is_reported() {
    let manifest = "\
[dependencies]
closeclaw-common = { path = \"../common\" }
closeclaw-llm = { path = \"../llm\" }
";
    let dependencies = regular_dependencies(manifest);
    let violations = workspace_internal_violations(&dependencies);
    assert_eq!(
        violations,
        vec!["closeclaw-llm"],
        "judgment must report the out-of-bound workspace crate by name"
    );
}

#[test]
fn test_dev_dependencies_are_not_judged() {
    let manifest = fs::read_to_string(manifest_path()).expect("read this crate's Cargo.toml");
    let dependencies = regular_dependencies(&manifest);
    assert!(
        !dependencies
            .iter()
            .any(|(name, _)| name == "tempfile" || name == "serial_test"),
        "only regular dependency entries may be judged"
    );
    let as_regular = "\
[dependencies]
closeclaw-session = { path = \"../session\" }
";
    let as_regular_dependencies = regular_dependencies(as_regular);
    assert_eq!(
        workspace_internal_violations(&as_regular_dependencies),
        vec!["closeclaw-session"],
        "the same judgment would flag a path dep outside the allowed set"
    );
}

#[test]
fn test_crates_io_dependencies_are_not_judged() {
    let manifest = "\
[dependencies]
tokio = { version = \"=1.35.0\", features = [\"full\"] }
serde_json = \"1.0\"
";
    assert!(
        workspace_internal_violations(&regular_dependencies(manifest)).is_empty(),
        "crates.io version deps must not participate in the judgment"
    );
}

#[test]
fn test_workspace_inheritance_dotted_key_is_judged_internal() {
    let manifest = "\
[dependencies]
closeclaw-llm.workspace = true
";
    let dependencies = regular_dependencies(manifest);
    assert_eq!(
        dependencies.len(),
        1,
        "the dotted key must resolve to a single dependency entry"
    );
    assert_eq!(dependencies[0].0, "closeclaw-llm");
    assert!(
        is_workspace_internal_declaration(&dependencies[0].1),
        "the `.workspace = true` form must not slip through as a crates.io dep"
    );
    assert_eq!(
        workspace_internal_violations(&dependencies),
        vec!["closeclaw-llm"]
    );
}

#[test]
fn test_inline_table_workspace_key_is_recognized_structurally() {
    let manifest = "\
[dependencies]
closeclaw-llm = { workspace = true }
";
    let dependencies = regular_dependencies(manifest);
    assert!(
        is_workspace_internal_declaration(&dependencies[0].1),
        "inline-table workspace inheritance is judged internal"
    );
    let substring_decoys = "\
[dependencies]
serde_json = { workspaces = \"path-to-llm\" }
tokio = { version = \"1.0\", features = [\"full\"] }
";
    assert!(
        workspace_internal_violations(&regular_dependencies(substring_decoys)).is_empty(),
        "keys must be identified structurally, not by substring matching"
    );
}

#[test]
fn test_trailing_comments_do_not_affect_internal_judgment() {
    let manifest = "\
[dependencies]
serde_json = \"1.0\" # path helper
tokio = { version = \"1.0\" } # workspace inheritance happens elsewhere
closeclaw-llm = { path = \"../llm\" } # out of bound
";
    assert_eq!(
        workspace_internal_violations(&regular_dependencies(manifest)),
        vec!["closeclaw-llm"],
        "comments must neither turn a crates.io dep internal nor hide a real path key"
    );
}

#[test]
fn test_parser_includes_target_dependencies_sections() {
    let manifest = "\
[target.'cfg(unix)'.dependencies]
closeclaw-llm = { path = \"../llm\" }

[target.'cfg(windows)'.dev-dependencies]
closeclaw-session = { path = \"../session\" }
";
    let dependencies = regular_dependencies(manifest);
    let violations = workspace_internal_violations(&dependencies);
    assert_eq!(
        violations,
        vec!["closeclaw-llm"],
        "target dependency sections must be judged; target dev sections must not"
    );
}

#[test]
fn test_sub_table_form_dependency_is_judged_internal() {
    let manifest = "\
[dependencies]
serde_json = \"1.0\"

[dependencies.closeclaw-llm]
version = \"1.0\"
path = \"../llm\"
";
    let dependencies = regular_dependencies(manifest);
    let llm = dependencies
        .iter()
        .find(|(name, _)| name == "closeclaw-llm")
        .expect("sub-table key lines must be extracted as the dependency's declaration");
    assert!(
        is_workspace_internal_declaration(&llm.1),
        "the `[dependencies.<name>]` sub-table form must be judged, not skipped"
    );
    assert_eq!(
        workspace_internal_violations(&dependencies),
        vec!["closeclaw-llm"]
    );
}

#[test]
fn test_target_sub_table_form_dependency_is_judged_internal() {
    let manifest = "\
[target.'cfg(unix)'.dependencies.closeclaw-llm]
path = \"../llm\"
";
    let dependencies = regular_dependencies(manifest);
    let violations = workspace_internal_violations(&dependencies);
    assert_eq!(
        violations,
        vec!["closeclaw-llm"],
        "the `[target.'…'.dependencies.<name>]` sub-table form must be judged, not skipped"
    );
}

#[test]
fn test_dev_sub_table_body_produces_no_entries() {
    let manifest = "\
[dev-dependencies.closeclaw-llm]
path = \"../llm\"
";
    assert!(
        regular_dependencies(manifest).is_empty(),
        "dev dependency sub-tables must not produce judged entries"
    );
}

#[test]
fn test_parser_strips_comments_in_dependency_entries() {
    let manifest = "\
[dependencies]
closeclaw-common = {
  path = \"../common\", # internal dep
}
serde_json = \"1.0\" # path helper
";
    let dependencies = regular_dependencies(manifest);
    let common = dependencies
        .iter()
        .find(|(name, _)| name == "closeclaw-common")
        .expect("multi-line tables with inline comments must parse into judged entries");
    assert!(
        is_workspace_internal_declaration(&common.1),
        "comment stripping must not hide a real path key"
    );
    assert!(
        workspace_internal_violations(&dependencies).is_empty(),
        "allowed path deps plus comment decoys must not be reported"
    );
}

#[test]
fn test_parser_extracts_only_regular_dependencies_section() {
    let manifest = "\
[package]
name = \"demo\"

[dependencies]
closeclaw-common = { path = \"../common\" }
serde_json = \"1.0\"

[dev-dependencies]
closeclaw-llm = { path = \"../llm\" }
tempfile = \"3\"
";
    let dependencies = regular_dependencies(manifest);
    let names: Vec<&str> = dependencies.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        vec!["closeclaw-common", "serde_json"],
        "only the regular dependency section feeds the judgment"
    );
}

#[test]
fn test_multi_line_inline_table_is_parsed_and_judged() {
    let manifest = "\
[dependencies]
closeclaw-common = {
  path = \"../common\"
}
serde_json = \"1.0\"
";
    let dependencies = regular_dependencies(manifest);
    let names: Vec<&str> = dependencies.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, vec!["closeclaw-common", "serde_json"]);
    assert!(
        workspace_internal_violations(&dependencies).is_empty(),
        "multi-line inline tables must be judged on structure"
    );
}
