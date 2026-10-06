//! Dependency boundary guard tests.
//!
//! Enforces the dependency allowed-edge table (docs/design/STANDARDS.md):
//! this crate's regular [dependencies] may only contain the workspace crates
//! {closeclaw-common, closeclaw-platform, closeclaw-debug-log}.

use std::fs;
use std::path::PathBuf;

const ALLOWED_WORKSPACE_DEPS: [&str; 3] = [
    "closeclaw-common",
    "closeclaw-platform",
    "closeclaw-debug-log",
];

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

fn workspace_internal_violations(dependencies: &[(String, String)]) -> Vec<String> {
    let mut violations: Vec<String> = dependencies
        .iter()
        .filter(|(_, declaration)| is_workspace_internal_declaration(declaration))
        .map(|(name, _)| name.clone())
        .filter(|name| !ALLOWED_WORKSPACE_DEPS.contains(&name.as_str()))
        .collect();
    violations.sort();
    violations.dedup();
    violations
}

fn is_workspace_internal_declaration(declaration: &str) -> bool {
    declaration.contains("path") || declaration.contains("workspace")
}

fn parse_regular_dependencies(manifest: &str) -> Vec<(String, String)> {
    let mut dependencies: Vec<(String, String)> = Vec::new();
    let mut pending: Option<(String, String)> = None;
    let mut in_dependencies_section = false;

    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            flush_pending(&mut pending, &mut dependencies);
            in_dependencies_section = trimmed == "[dependencies]";
        } else if in_dependencies_section && is_entry_line(trimmed) {
            extend_or_push_entry(trimmed, &mut pending, &mut dependencies);
        }
    }
    flush_pending(&mut pending, &mut dependencies);
    dependencies
}

fn flush_pending(pending: &mut Option<(String, String)>, dependencies: &mut Vec<(String, String)>) {
    if let Some(entry) = pending.take() {
        dependencies.push(entry);
    }
}

fn is_entry_line(trimmed: &str) -> bool {
    !trimmed.is_empty() && !trimmed.starts_with('#')
}

fn extend_or_push_entry(
    trimmed: &str,
    pending: &mut Option<(String, String)>,
    dependencies: &mut Vec<(String, String)>,
) {
    let (name, declaration) = match pending.take() {
        Some((name, declaration)) => {
            let mut joined = declaration;
            joined.push(' ');
            joined.push_str(trimmed);
            (name, joined)
        }
        None => match trimmed
            .split_once('=')
            .filter(|(raw_name, _)| is_dependency_name(raw_name.trim()))
        {
            Some((raw_name, declaration)) => {
                (raw_name.trim().to_string(), declaration.trim().to_string())
            }
            None => return,
        },
    };
    if braces_balanced(&declaration) {
        dependencies.push((name, declaration));
    } else {
        pending.replace((name, declaration));
    }
}

fn is_dependency_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn braces_balanced(declaration: &str) -> bool {
    declaration.matches('{').count() == declaration.matches('}').count()
}

#[test]
fn test_regular_dependencies_stay_within_allowed_edge_table() {
    let manifest = fs::read_to_string(manifest_path()).expect("read this crate's Cargo.toml");
    let dependencies = parse_regular_dependencies(&manifest);
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
fn test_out_of_bound_internal_dependency_is_reported() {
    let dependencies = vec![
        (
            "closeclaw-common".to_string(),
            r#"{ path = "../common" }"#.to_string(),
        ),
        (
            "closeclaw-llm".to_string(),
            r#"{ path = "../llm" }"#.to_string(),
        ),
    ];
    let violations = workspace_internal_violations(&dependencies);
    assert_eq!(
        violations,
        vec!["closeclaw-llm".to_string()],
        "judgment must report the out-of-bound workspace crate by name"
    );
}

#[test]
fn test_dev_dependencies_are_not_judged() {
    let manifest = fs::read_to_string(manifest_path()).expect("read this crate's Cargo.toml");
    let dependencies = parse_regular_dependencies(&manifest);
    assert!(
        !dependencies
            .iter()
            .any(|(name, _)| name == "closeclaw-im-adapter" || name == "tempfile"),
        "only [dependencies] entries may be judged"
    );
    let dev_only = vec![(
        "closeclaw-im-adapter".to_string(),
        r#"{ path = "../im_adapter" }"#.to_string(),
    )];
    assert_eq!(
        workspace_internal_violations(&dev_only),
        vec!["closeclaw-im-adapter".to_string()],
        "the same judgment would flag a path dep outside the allowed set"
    );
}

#[test]
fn test_registry_dependencies_are_not_judged() {
    let dependencies = vec![
        (
            "tokio".to_string(),
            r#"{ version = "=1.35.0", features = ["full"] }"#.to_string(),
        ),
        ("serde".to_string(), "\"1.0\"".to_string()),
    ];
    assert!(
        workspace_internal_violations(&dependencies).is_empty(),
        "crates.io version deps must not participate in the judgment"
    );
}

#[test]
fn test_parser_extracts_only_regular_dependencies_section() {
    let manifest = "\
[package]
name = \"demo\"

[dependencies]
closeclaw-common = { path = \"../common\" }
serde = \"1.0\"

[dev-dependencies]
closeclaw-llm = { path = \"../llm\" }
tempfile = \"3\"
";
    let dependencies = parse_regular_dependencies(manifest);
    let names: Vec<&str> = dependencies.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, vec!["closeclaw-common", "serde"]);
}

#[test]
fn test_parser_joins_multi_line_inline_tables() {
    let manifest = "\
[dependencies]
closeclaw-platform = {
  path = \"../platform\"
}
serde = \"1.0\"
";
    let dependencies = parse_regular_dependencies(manifest);
    let names: Vec<&str> = dependencies.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, vec!["closeclaw-platform", "serde"]);
}
