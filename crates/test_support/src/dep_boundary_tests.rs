//! Unit tests for the shared dependency boundary guard.
//!
//! The parser behavior cases are merged from the five hand-written
//! `dependency_boundary_tests.rs` files (slash, processor_chain, session,
//! tools, gateway) that this crate replaces, deduplicated, and re-targeted
//! at the real `toml` grammar implementation. Two cases are explicit
//! regressions for the common defects of the hand-rolled scanners
//! (issue #3424): look-alike sections misjudged as regular dependencies,
//! and quoted target keys containing dots failing open.

use std::path::PathBuf;

use tempfile::TempDir;

use crate::dep_boundary::{
    assert_workspace_dep_boundary, is_workspace_internal_declaration, manifest_dependencies,
    regular_dependencies, workspace_internal_violations, DepBoundarySpec, DependencySection,
};

#[test]
fn test_allowed_workspace_deps_pass_the_boundary() {
    let manifest = r#"
[dependencies]
closeclaw-common = { path = "../common" }
closeclaw-debug-log = { version = "0.1", path = "../debug_log", features = ["pretty"] }
serde = "1.0"
"#;
    let entries = regular_dependencies(manifest).expect("valid TOML manifest");
    assert_eq!(entries.len(), 3);
    let allowed = ["closeclaw-common", "closeclaw-debug-log"];
    assert!(
        workspace_internal_violations(&entries, &allowed).is_empty(),
        "whitelisted path deps must pass the boundary judgment"
    );
}

#[test]
fn test_manifest_dependencies_classify_every_section_kind() {
    let manifest = r#"
[dependencies]
closeclaw-common = { path = "../common" }

[target.'cfg(unix)'.dependencies]
closeclaw-debug-log = { path = "../debug_log" }

[dev-dependencies]
tempfile = "3.10"

[target.'cfg(windows)'.dev-dependencies]
serial_test = "3"

[build-dependencies]
cc = "1"

[target.'cfg(windows)'.build-dependencies]
winapi = "0.3"
"#;
    let entries = manifest_dependencies(manifest).expect("valid TOML manifest");
    let section_of = |name: &str| {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.section)
            .unwrap_or_else(|| panic!("entry {name} must be collected"))
    };
    assert_eq!(section_of("closeclaw-common"), DependencySection::Regular);
    assert_eq!(section_of("closeclaw-debug-log"), DependencySection::Target);
    assert_eq!(section_of("tempfile"), DependencySection::Dev);
    assert_eq!(section_of("serial_test"), DependencySection::Dev);
    assert_eq!(section_of("cc"), DependencySection::Build);
    assert_eq!(section_of("winapi"), DependencySection::Build);
}

/// Regression for issue #3424 defect 1: the loose scanner faction judged any
/// header ending in `.dependencies` as a regular section, so look-alike
/// tables leaked entries into the judgment. The `toml` crate reads them as
/// unrelated tables, so no dependency entry may be produced from them.
#[test]
fn test_look_alike_sections_produce_no_dependency_entries() {
    let manifest = r#"
[workspace.dependencies]
closeclaw-session = { path = "../session" }

[workspace.dependencies.closeclaw-skills]
path = "../skills"

[package.metadata.dependencies]
closeclaw-workflow = { path = "../workflow" }
"#;
    let entries = manifest_dependencies(manifest).expect("valid TOML manifest");
    assert!(
        entries.is_empty(),
        "look-alike sections must not be classified as dependency sections: {entries:?}"
    );
    assert!(regular_dependencies(manifest)
        .expect("valid TOML manifest")
        .is_empty());
    assert!(workspace_internal_violations(&entries, &[]).is_empty());
}

/// Regression for issue #3424 defect 2: a quoted target key containing a dot
/// (`'cfg(unix).x'` is a single TOML key) made the scanners split the header
/// on `.` and drop the section, so its entries silently escaped the
/// judgment. The `toml` crate resolves the quoted key, so the entries must
/// be collected and judged.
#[test]
fn test_quoted_target_key_with_dot_is_collected_and_judged() {
    let manifest = r#"
[target.'cfg(unix).x'.dependencies]
closeclaw-session = { path = "../session" }
"#;
    let entries = regular_dependencies(manifest).expect("valid TOML manifest");
    assert_eq!(
        entries.len(),
        1,
        "a quoted target key containing dots must not fail open"
    );
    assert_eq!(entries[0].name, "closeclaw-session");
    assert_eq!(entries[0].section, DependencySection::Target);
    assert!(is_workspace_internal_declaration(&entries[0].declaration));
    assert_eq!(
        workspace_internal_violations(&entries, &[]),
        vec!["closeclaw-session".to_string()],
        "entries under a quoted target key containing dots must be judged"
    );
}

/// Workspace-internal recognition is structural (`path` key or
/// `workspace = true` inheritance), never substring matching; string and
/// features-table declarations are crates.io deps.
#[test]
fn test_workspace_internal_declaration_is_structural() {
    let internal = r#"
[dependencies]
closeclaw-common = { path = "../common" }
closeclaw-debug-log = { version = "0.1", workspace = true }
"#;
    let entries = regular_dependencies(internal).expect("valid TOML manifest");
    assert!(
        entries
            .iter()
            .all(|entry| is_workspace_internal_declaration(&entry.declaration)),
        "path and workspace-inheritance declarations are judged internal"
    );

    let external = r#"
[dependencies]
serde = { workspaces = "path-to-config" }
tokio = { version = "1.0", features = ["full"] }
serde_json = "1.0"
"#;
    let entries = regular_dependencies(external).expect("valid TOML manifest");
    assert!(
        entries
            .iter()
            .all(|entry| !is_workspace_internal_declaration(&entry.declaration)),
        "crates.io version deps must not participate in the judgment"
    );
}

#[test]
fn test_workspace_inheritance_dotted_key_is_judged_internal() {
    let manifest = r#"
[dependencies]
closeclaw-session.workspace = true
"#;
    let entries = regular_dependencies(manifest).expect("valid TOML manifest");
    assert_eq!(
        entries.len(),
        1,
        "the dotted key must resolve to one dependency entry"
    );
    assert_eq!(entries[0].name, "closeclaw-session");
    assert!(
        is_workspace_internal_declaration(&entries[0].declaration),
        "the `.workspace = true` form must not slip through as a crates.io dep"
    );
    assert_eq!(
        workspace_internal_violations(&entries, &[]),
        vec!["closeclaw-session".to_string()]
    );
}

#[test]
fn test_violations_are_reported_sorted_and_deduplicated() {
    let manifest = r#"
[dependencies]
closeclaw-common = { path = "../common" }
closeclaw-workflow = { path = "../workflow" }
closeclaw-memory = { path = "../memory" }

[target.'cfg(unix)'.dependencies]
closeclaw-memory = { path = "../memory" }
"#;
    let entries = regular_dependencies(manifest).expect("valid TOML manifest");
    assert_eq!(
        workspace_internal_violations(&entries, &["closeclaw-common"]),
        vec![
            "closeclaw-memory".to_string(),
            "closeclaw-workflow".to_string()
        ],
        "out-of-bound crates must be reported by name, sorted, once per crate"
    );
}

#[test]
fn test_sub_table_form_dependency_is_judged_internal() {
    let manifest = r#"
[dependencies]
serde = "1.0"

[dependencies.closeclaw-session]
version = "1.0"
path = "../session"
"#;
    let entries = regular_dependencies(manifest).expect("valid TOML manifest");
    let session = entries
        .iter()
        .find(|entry| entry.name == "closeclaw-session")
        .expect("the [dependencies.<name>] sub-table form must yield an entry");
    assert!(is_workspace_internal_declaration(&session.declaration));
    assert_eq!(
        workspace_internal_violations(&entries, &[]),
        vec!["closeclaw-session".to_string()],
        "the `[dependencies.<name>]` sub-table form must be judged, not skipped"
    );
}

#[test]
fn test_target_sub_table_form_dependency_is_judged_internal() {
    let manifest = r#"
[target.'cfg(unix)'.dependencies.closeclaw-session]
path = "../session"
"#;
    let entries = regular_dependencies(manifest).expect("valid TOML manifest");
    assert_eq!(
        workspace_internal_violations(&entries, &[]),
        vec!["closeclaw-session".to_string()],
        "the `[target.'…'.dependencies.<name>]` sub-table form must be judged, not skipped"
    );
}

#[test]
fn test_dev_and_build_sections_are_exempt_from_the_boundary() {
    let manifest = r#"
[dev-dependencies]
closeclaw-session = { path = "../session" }

[dev-dependencies.closeclaw-workflow]
path = "../workflow"

[build-dependencies]
cc = "1"

[target.'cfg(windows)'.dev-dependencies]
closeclaw-skills = { path = "../skills" }

[target.'cfg(windows)'.build-dependencies]
winapi = "0.3"
"#;
    let entries = manifest_dependencies(manifest).expect("valid TOML manifest");
    assert_eq!(
        entries.len(),
        5,
        "every dev/build entry must still be collected for classification"
    );
    assert!(
        entries.iter().all(|entry| !entry.section.is_regular()),
        "dev and build entries must stay out of the boundary judgment: {entries:?}"
    );
    assert!(workspace_internal_violations(&entries, &[]).is_empty());
}

#[test]
fn test_trailing_comments_do_not_affect_internal_judgment() {
    let manifest = r#"
[dependencies]
serde = "1.0" # path helper
tokio = { version = "1.0" } # workspace inheritance happens elsewhere
closeclaw-common = {
  path = "../common", # internal dep
}
"#;
    let entries = regular_dependencies(manifest).expect("valid TOML manifest");
    assert!(
        workspace_internal_violations(&entries, &["closeclaw-common"]).is_empty(),
        "comments must neither turn a crates.io dep internal nor hide a real path key"
    );
}

/// Quoted dependency keys (`"serde"` / `'serde'`) are valid TOML and must be
/// collected under their unquoted package name.
#[test]
fn test_quoted_dependency_keys_are_collected_by_unquoted_name() {
    let manifest = r#"
[dependencies]
"closeclaw-common" = { path = "../common" }
'serde_json' = "1.0"
"#;
    let entries = regular_dependencies(manifest).expect("valid TOML manifest");
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec!["closeclaw-common", "serde_json"]);
    assert!(workspace_internal_violations(&entries, &["closeclaw-common"]).is_empty());
}

#[test]
fn test_multi_line_inline_table_is_parsed_and_judged() {
    let manifest = r#"
[dependencies]
closeclaw-debug-log = {
  path = "../debug_log"
}
serde = "1.0"
"#;
    let entries = regular_dependencies(manifest).expect("valid TOML manifest");
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec!["closeclaw-debug-log", "serde"]);
    assert!(
        workspace_internal_violations(&entries, &["closeclaw-debug-log"]).is_empty(),
        "multi-line tables must be judged on structure"
    );
}

#[test]
fn test_empty_manifest_yields_no_dependencies() {
    let entries = manifest_dependencies("").expect("an empty manifest is valid TOML");
    assert!(entries.is_empty());
}

#[test]
fn test_malformed_manifest_yields_manifest_error() {
    let error = manifest_dependencies("[dependencies").expect_err("unclosed table header");
    assert!(
        error.to_string().contains("invalid TOML manifest"),
        "the error must point at manifest parsing: {error}"
    );
}

#[test]
fn test_manifest_without_dependency_section_yields_no_entries() {
    let manifest = r#"
[package]
name = "demo"
"#;
    let entries = manifest_dependencies(manifest).expect("valid TOML manifest");
    assert!(entries.is_empty());
    assert!(regular_dependencies(manifest)
        .expect("valid TOML manifest")
        .is_empty());
}

#[test]
fn test_assert_entry_accepts_this_crate_manifest() {
    let spec = DepBoundarySpec::new(
        "closeclaw-test-support",
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    );
    assert_workspace_dep_boundary(&spec);
}

#[test]
#[should_panic(expected = "workspace deps outside the allowed edge table")]
fn test_assert_entry_reports_out_of_bound_violation() {
    let manifest = r#"
[dependencies]
closeclaw-common = { path = "../common" }
closeclaw-memory = { path = "../memory" }
"#;
    let (_dir, spec) = synthetic_spec(manifest);
    let spec = spec.allowed_workspace_deps(&["closeclaw-common"]);
    assert_workspace_dep_boundary(&spec);
}

/// A malformed manifest must fail the guard loudly, never pass it silently.
#[test]
#[should_panic(expected = "cannot parse manifest")]
fn test_assert_entry_fails_on_malformed_manifest() {
    let (_dir, spec) = synthetic_spec("[dependencies");
    assert_workspace_dep_boundary(&spec);
}

/// Zero extracted regular entries must fail the guard: that is exactly the
/// fail-open shape the hand-rolled scanners produced on look-alike sections.
#[test]
#[should_panic(expected = "parser must extract the [dependencies] entries")]
fn test_assert_entry_fails_without_any_regular_dependency() {
    let (_dir, spec) = synthetic_spec("[package]\nname = \"demo\"\n");
    assert_workspace_dep_boundary(&spec);
}

/// Dev-only semantics (aligned with the original slash/session judgments):
/// a test-only workspace crate may live in [dev-dependencies] without
/// tripping the boundary.
#[test]
fn test_dev_only_crate_in_dev_section_is_accepted() {
    let manifest = r#"
[dependencies]
closeclaw-common = { path = "../common" }

[dev-dependencies]
closeclaw-session = { path = "../session" }
"#;
    let (_dir, spec) = synthetic_spec(manifest);
    let spec = spec
        .allowed_workspace_deps(&["closeclaw-common"])
        .dev_only_workspace_deps(&["closeclaw-session"]);
    assert_workspace_dep_boundary(&spec);
}

#[test]
#[should_panic(expected = "must not appear in regular dependencies")]
fn test_dev_only_crate_leaked_into_regular_dependencies_is_rejected() {
    let manifest = r#"
[dependencies]
closeclaw-session = { path = "../session" }

[dev-dependencies]
closeclaw-session = { path = "../session" }
"#;
    let (_dir, spec) = synthetic_spec(manifest);
    let spec = spec
        .allowed_workspace_deps(&["closeclaw-common"])
        .dev_only_workspace_deps(&["closeclaw-session"]);
    assert_workspace_dep_boundary(&spec);
}

/// Writes the manifest into a temp dir (system temp per STANDARDS §8) and
/// returns the dir so it outlives the spec's manifest path.
fn synthetic_spec(manifest: &str) -> (TempDir, DepBoundarySpec) {
    let dir = TempDir::new().expect("create temp dir for the synthetic manifest");
    let path = dir.path().join("Cargo.toml");
    std::fs::write(&path, manifest).expect("write synthetic manifest");
    let spec = DepBoundarySpec::new("synthetic-crate", path);
    (dir, spec)
}
