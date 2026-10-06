//! Dependency boundary guard tests.
//!
//! Enforces the dependency allowed-edge table (docs/design/STANDARDS.md):
//! this crate's regular dependency entries — key entries in [dependencies] /
//! [target.'…'.dependencies] and their `[dependencies.<name>]` /
//! `[target.'…'.dependencies.<name>]` sub-tables —
//! may only contain the workspace crates
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

fn workspace_internal_violations<'a>(dependencies: &[(&'a str, &'a str)]) -> Vec<&'a str> {
    let mut violations: Vec<&'a str> = dependencies
        .iter()
        .filter(|(_, declaration)| is_workspace_internal_declaration(declaration))
        .map(|(name, _)| *name)
        .filter(|name| !ALLOWED_WORKSPACE_DEPS.contains(name))
        .collect();
    violations.sort_unstable();
    violations.dedup();
    violations
}

/// A declaration is workspace-internal only through TOML structure: a `path`
/// key, or `workspace = true` inheritance — recognized by key, never by
/// substring matching. Covers inline tables plus the dotted-key and sub-table
/// forms, whose key lines arrive joined into a plain `key = value` list.
/// Trailing comments are stripped before the judgment.
fn is_workspace_internal_declaration(declaration: &str) -> bool {
    let declaration = strip_trailing_comment(declaration).trim();
    let content = inline_table_inner(declaration).unwrap_or(declaration);
    inline_table_items(content)
        .into_iter()
        .any(|(key, value)| key == "path" || (key == "workspace" && value == "true"))
}

/// Quote/escape scan state shared by the TOML scanners: basic strings
/// (`"…"` with `\` escapes) and literal strings (`'…'`) hide structural
/// characters such as `#`, `,`, and brackets from the scanners.
struct StringScan {
    in_basic_string: bool,
    in_literal_string: bool,
    escaped: bool,
}

impl StringScan {
    fn new() -> Self {
        Self {
            in_basic_string: false,
            in_literal_string: false,
            escaped: false,
        }
    }

    /// Consumes one char: string delimiters and contents only advance the
    /// state and yield `None`; chars outside strings are structural and are
    /// yielded as-is.
    fn feed(&mut self, ch: char) -> Option<char> {
        if self.in_basic_string {
            if self.escaped {
                self.escaped = false;
            } else if ch == '\\' {
                self.escaped = true;
            } else if ch == '"' {
                self.in_basic_string = false;
            }
            return None;
        }
        if self.in_literal_string {
            if ch == '\'' {
                self.in_literal_string = false;
            }
            return None;
        }
        match ch {
            '"' => {
                self.in_basic_string = true;
                None
            }
            '\'' => {
                self.in_literal_string = true;
                None
            }
            structural => Some(structural),
        }
    }
}

/// Removes a trailing comment: the first `#` outside of quotes starts one.
fn strip_trailing_comment(line: &str) -> &str {
    let mut scan = StringScan::new();
    for (index, ch) in line.char_indices() {
        if scan.feed(ch) == Some('#') {
            return &line[..index];
        }
    }
    line
}

fn inline_table_inner(declaration: &str) -> Option<&str> {
    declaration
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .map(str::trim)
}

/// Splits inline-table content into top-level `(key, value)` items, ignoring
/// separators inside strings, arrays, and nested tables.
fn inline_table_items(inner: &str) -> Vec<(&str, &str)> {
    let mut item_slices: Vec<&str> = Vec::new();
    let mut scan = StringScan::new();
    let mut depth = 0usize;
    let mut item_start = 0;
    for (index, ch) in inner.char_indices() {
        match scan.feed(ch) {
            Some('[') | Some('{') => depth += 1,
            Some(']') | Some('}') => depth = depth.saturating_sub(1),
            Some(',') if depth == 0 => {
                item_slices.push(&inner[item_start..index]);
                item_start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    item_slices.push(&inner[item_start..]);
    item_slices
        .into_iter()
        .filter_map(|item| {
            let (key, value) = item.split_once('=')?;
            Some((key.trim().trim_matches('"'), value.trim()))
        })
        .collect()
}

fn parse_regular_dependencies(manifest: &str) -> Vec<(String, String)> {
    let mut dependencies: Vec<(String, String)> = Vec::new();
    let mut pending: Option<(String, String)> = None;
    let mut section = Section::Other;

    for line in manifest.lines() {
        let trimmed = strip_trailing_comment(line).trim();
        if trimmed.starts_with('[') {
            flush_pending(&mut pending, &mut dependencies);
            flush_sub_table(&mut section, &mut dependencies);
            section = classify_section(trimmed);
        } else if is_entry_line(trimmed) {
            match &mut section {
                Section::Regular => {
                    extend_or_push_entry(trimmed, &mut pending, &mut dependencies);
                }
                Section::SubTable { keys, .. } => {
                    if trimmed.contains('=') {
                        keys.push(trimmed.to_string());
                    }
                }
                Section::Other => {}
            }
        }
    }
    flush_pending(&mut pending, &mut dependencies);
    flush_sub_table(&mut section, &mut dependencies);
    dependencies
}

/// Parser state between manifest lines: which section the cursor is in, and —
/// for the sub-table form — the dependency name plus its collected `key =
/// value` lines.
enum Section {
    Regular,
    SubTable { name: String, keys: Vec<String> },
    Other,
}

/// Classifies a section header: `[dependencies]` and
/// `[target.'…'.dependencies]` are regular dependency sections, a header
/// ending in `dependencies.<name>` opens the sub-table form of dependency
/// `<name>`; everything else (dev, build, package, …) stays out of scope.
fn classify_section(header: &str) -> Section {
    let Some(inner) = header
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return Section::Other;
    };
    let mut segments = inner.trim().split('.').map(str::trim).rev();
    match (segments.next(), segments.next()) {
        (Some("dependencies"), _) => Section::Regular,
        (Some(name), Some("dependencies")) => Section::SubTable {
            name: name.trim_matches('"').to_string(),
            keys: Vec::new(),
        },
        _ => Section::Other,
    }
}

fn flush_sub_table(section: &mut Section, dependencies: &mut Vec<(String, String)>) {
    if let Section::SubTable { name, keys } = std::mem::replace(section, Section::Other) {
        if !keys.is_empty() {
            dependencies.push((name, keys.join(", ")));
        }
    }
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
            Some((raw_name, declaration)) => normalize_entry(raw_name.trim(), declaration.trim()),
            None => return,
        },
    };
    if braces_balanced(&declaration) {
        dependencies.push((name, declaration));
    } else {
        pending.replace((name, declaration));
    }
}

/// Resolves a possibly dotted entry key: `name.workspace = true` declares the
/// dependency `name` with workspace inheritance, so the sub-key belongs to
/// the declaration, not the dependency name.
fn normalize_entry(raw_name: &str, declaration: &str) -> (String, String) {
    match raw_name.split_once('.') {
        Some((name, subkey)) => (name.to_string(), format!("{subkey} = {declaration}")),
        None => (raw_name.to_string(), declaration.to_string()),
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
    let violations = workspace_internal_violations(&as_ref_pairs(&dependencies));
    assert!(
        violations.is_empty(),
        "workspace deps outside the allowed edge table: {violations:?}"
    );
}

#[test]
fn test_out_of_bound_internal_dependency_is_reported() {
    let dependencies = vec![
        ("closeclaw-common", r#"{ path = "../common" }"#),
        ("closeclaw-llm", r#"{ path = "../llm" }"#),
    ];
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
    let dependencies = parse_regular_dependencies(&manifest);
    assert!(
        !dependencies
            .iter()
            .any(|(name, _)| name == "closeclaw-im-adapter" || name == "tempfile"),
        "only regular dependency entries may be judged"
    );
    let dev_only = vec![("closeclaw-im-adapter", r#"{ path = "../im_adapter" }"#)];
    assert_eq!(
        workspace_internal_violations(&dev_only),
        vec!["closeclaw-im-adapter"],
        "the same judgment would flag a path dep outside the allowed set"
    );
}

#[test]
fn test_crates_io_dependencies_are_not_judged() {
    let dependencies = vec![
        ("tokio", r#"{ version = "=1.35.0", features = ["full"] }"#),
        ("serde", "\"1.0\""),
    ];
    assert!(
        workspace_internal_violations(&dependencies).is_empty(),
        "crates.io version deps must not participate in the judgment"
    );
}

#[test]
fn test_workspace_inheritance_dotted_key_is_judged_internal() {
    let manifest = "\
[dependencies]
closeclaw-llm.workspace = true
";
    let dependencies = parse_regular_dependencies(manifest);
    assert_eq!(
        dependencies,
        vec![("closeclaw-llm".to_string(), "workspace = true".to_string())],
        "the dotted key must resolve to the dependency name with a sub-key declaration"
    );
    assert_eq!(
        workspace_internal_violations(&as_ref_pairs(&dependencies)),
        vec!["closeclaw-llm"],
        "the `.workspace = true` form must not slip through as a crates.io dep"
    );
}

#[test]
fn test_inline_table_workspace_key_is_recognized_structurally() {
    assert!(
        is_workspace_internal_declaration(r#"{ workspace = true }"#),
        "inline-table workspace inheritance is judged internal"
    );
    let substring_decoys = vec![
        ("serde", r#"{ workspaces = "path-to-llm" }"#),
        ("tokio", r#"{ version = "1.0", features = ["full"] }"#),
    ];
    assert!(
        workspace_internal_violations(&substring_decoys).is_empty(),
        "keys must be identified structurally, not by substring matching"
    );
}

#[test]
fn test_trailing_comments_do_not_affect_internal_judgment() {
    let crates_io_with_comments = vec![
        ("serde", "\"1.0\" # path helper"),
        (
            "tokio",
            r#"{ version = "1.0" } # workspace inheritance happens elsewhere"#,
        ),
    ];
    assert!(
        workspace_internal_violations(&crates_io_with_comments).is_empty(),
        "keywords inside trailing comments must not turn a crates.io dep internal"
    );
    let path_dep_with_comment = vec![("closeclaw-llm", r#"{ path = "../llm" } # out of bound"#)];
    assert_eq!(
        workspace_internal_violations(&path_dep_with_comment),
        vec!["closeclaw-llm"],
        "comment stripping must not hide a real path key"
    );
}

#[test]
fn test_parser_includes_target_dependencies_sections() {
    let manifest = "\
[target.'cfg(unix)'.dependencies]
closeclaw-llm = { path = \"../llm\" }

[target.'cfg(windows)'.dev-dependencies]
closeclaw-im-adapter = { path = \"../im_adapter\" }
";
    let dependencies = parse_regular_dependencies(manifest);
    assert_eq!(
        workspace_internal_violations(&as_ref_pairs(&dependencies)),
        vec!["closeclaw-llm"],
        "target dependency sections must be judged; target dev sections must not"
    );
}

#[test]
fn test_sub_table_form_dependency_is_judged_internal() {
    let manifest = "\
[dependencies]
serde = \"1.0\"

[dependencies.closeclaw-llm]
version = \"1.0\"
path = \"../llm\"
";
    let dependencies = parse_regular_dependencies(manifest);
    assert_eq!(
        dependencies,
        vec![
            ("serde".to_string(), "\"1.0\"".to_string()),
            (
                "closeclaw-llm".to_string(),
                "version = \"1.0\", path = \"../llm\"".to_string()
            ),
        ],
        "sub-table key lines must be extracted as the dependency's declaration"
    );
    assert_eq!(
        workspace_internal_violations(&as_ref_pairs(&dependencies)),
        vec!["closeclaw-llm"],
        "the `[dependencies.<name>]` sub-table form must be judged, not skipped"
    );
}

#[test]
fn test_target_sub_table_form_dependency_is_judged_internal() {
    let manifest = "\
[target.'cfg(unix)'.dependencies.closeclaw-llm]
path = \"../llm\"
";
    let dependencies = parse_regular_dependencies(manifest);
    assert_eq!(
        dependencies,
        vec![("closeclaw-llm".to_string(), "path = \"../llm\"".to_string())],
        "target sub-table key lines must be extracted as the dependency's declaration"
    );
    assert_eq!(
        workspace_internal_violations(&as_ref_pairs(&dependencies)),
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
    let dependencies = parse_regular_dependencies(manifest);
    assert!(
        dependencies.is_empty(),
        "dev dependency sub-tables must not produce judged entries"
    );
}

#[test]
fn test_parser_strips_comments_in_dependency_entries() {
    let manifest = "\
[dependencies]
closeclaw-platform = {
  path = \"../platform\", # internal dep
}
serde = \"1.0\" # path helper
";
    let dependencies = parse_regular_dependencies(manifest);
    assert_eq!(
        dependencies,
        vec![
            (
                "closeclaw-platform".to_string(),
                r#"{ path = "../platform", }"#.to_string()
            ),
            ("serde".to_string(), "\"1.0\"".to_string()),
        ],
        "trailing comments must be stripped before entry joining and judgment"
    );
    assert!(
        workspace_internal_violations(&as_ref_pairs(&dependencies)).is_empty(),
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

fn as_ref_pairs(dependencies: &[(String, String)]) -> Vec<(&str, &str)> {
    dependencies
        .iter()
        .map(|(name, declaration)| (name.as_str(), declaration.as_str()))
        .collect()
}
