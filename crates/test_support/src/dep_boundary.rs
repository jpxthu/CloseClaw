//! Shared dependency-boundary guard for CloseClaw workspace crates.
//!
//! Parses a crate's real `Cargo.toml` with the `toml` crate and judges its
//! workspace-internal regular dependencies against a per-crate allow list
//! (docs/design/STANDARDS.md §依赖方向允许边表). Inline tables, sub-tables,
//! dotted keys, quoted target keys, multi-line tables, and comments are all
//! handled by the real TOML grammar instead of hand-rolled scanners.
//!
//! Judgment scope: the `[dependencies]` section plus every
//! `[target.'…'.dependencies]` section (including their `[….<name>]`
//! sub-table forms). `dev-dependencies` and `build-dependencies` — declared
//! at top level or under a `[target.'…']` table — are classified but exempt
//! from the boundary judgment.

use std::fmt;
use std::path::PathBuf;

use toml::Value;

/// Manifest sections that declare dependencies, classified from the TOML
/// structure of the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencySection {
    /// `[dependencies]`
    Regular,
    /// `[target.'…'.dependencies]` — quoted target keys containing dots are
    /// single keys of the `[target]` table, so they resolve correctly.
    Target,
    /// `[dev-dependencies]` and `[target.'…'.dev-dependencies]`
    Dev,
    /// `[build-dependencies]` and `[target.'…'.build-dependencies]`
    Build,
}

impl DependencySection {
    /// Regular sections feed the boundary judgment; dev and build sections
    /// are exempt (they do not affect the published module boundaries).
    pub fn is_regular(self) -> bool {
        matches!(self, Self::Regular | Self::Target)
    }
}

/// One dependency entry extracted from a manifest.
#[derive(Debug, Clone)]
pub struct DependencyEntry {
    /// Dependency name (package name) as written in the manifest.
    pub name: String,
    /// Section the dependency was declared in.
    pub section: DependencySection,
    /// Raw TOML declaration: a version string, or a table (inline,
    /// multi-line, dotted-key, or `[dependencies.<name>]` sub-table form).
    pub declaration: Value,
}

/// Error produced when a manifest is not valid TOML.
#[derive(Debug)]
pub struct ManifestError(String);

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ManifestError {}

/// All dependency entries of a manifest, classified by section.
///
/// A manifest without any dependency section yields an empty list; a
/// manifest that is not valid TOML yields [`ManifestError`].
pub fn manifest_dependencies(manifest: &str) -> Result<Vec<DependencyEntry>, ManifestError> {
    let parsed: Value = toml::from_str(manifest)
        .map_err(|error| ManifestError(format!("invalid TOML manifest: {error}")))?;
    let mut entries = Vec::new();
    collect_top_level_sections(&parsed, &mut entries);
    collect_target_sections(&parsed, &mut entries);
    Ok(entries)
}

/// Regular dependency entries of a manifest: `[dependencies]` plus every
/// `[target.'…'.dependencies]`. Dev, build, and look-alike sections such as
/// `[workspace.dependencies]` stay out of the result.
pub fn regular_dependencies(manifest: &str) -> Result<Vec<DependencyEntry>, ManifestError> {
    Ok(manifest_dependencies(manifest)?
        .into_iter()
        .filter(|entry| entry.section.is_regular())
        .collect())
}

/// A declaration is workspace-internal only through TOML structure: a `path`
/// key, or `workspace = true` inheritance — recognized by key, never by
/// substring matching. String declarations are crates.io version deps.
pub fn is_workspace_internal_declaration(declaration: &Value) -> bool {
    declaration.as_table().is_some_and(|table| {
        table.contains_key("path") || table.get("workspace") == Some(&Value::Boolean(true))
    })
}

/// Names of workspace-internal regular dependencies outside the allowed
/// edge table, sorted and deduplicated.
pub fn workspace_internal_violations(
    dependencies: &[DependencyEntry],
    allowed_workspace_deps: &[&str],
) -> Vec<String> {
    let mut violations: Vec<String> = dependencies
        .iter()
        .filter(|entry| entry.section.is_regular())
        .filter(|entry| is_workspace_internal_declaration(&entry.declaration))
        .map(|entry| entry.name.clone())
        .filter(|name| !allowed_workspace_deps.contains(&name.as_str()))
        .collect();
    violations.sort_unstable();
    violations.dedup();
    violations
}

/// Declarative boundary specification for one crate, consumed by
/// [`assert_workspace_dep_boundary`].
#[derive(Debug, Clone)]
pub struct DepBoundarySpec {
    /// Crate name used in failure messages.
    pub crate_name: &'static str,
    /// Path to the crate's real `Cargo.toml` (typically built from
    /// `env!("CARGO_MANIFEST_DIR")`).
    pub manifest_path: PathBuf,
    /// Workspace crates allowed in regular dependency sections.
    pub allowed_workspace_deps: &'static [&'static str],
    /// Workspace crates that must stay dev-only: they may live in
    /// `[dev-dependencies]` but never in regular dependency sections.
    pub dev_only_workspace_deps: &'static [&'static str],
}

impl DepBoundarySpec {
    /// Builds a spec for the crate named `crate_name` with its manifest at
    /// `manifest_path`; attach allow lists via the builder methods.
    pub fn new(crate_name: &'static str, manifest_path: impl Into<PathBuf>) -> Self {
        Self {
            crate_name,
            manifest_path: manifest_path.into(),
            allowed_workspace_deps: &[],
            dev_only_workspace_deps: &[],
        }
    }

    /// Sets the workspace crates allowed in regular dependency sections.
    pub fn allowed_workspace_deps(mut self, deps: &'static [&'static str]) -> Self {
        self.allowed_workspace_deps = deps;
        self
    }

    /// Sets the workspace crates that must remain dev-only.
    pub fn dev_only_workspace_deps(mut self, deps: &'static [&'static str]) -> Self {
        self.dev_only_workspace_deps = deps;
        self
    }
}

/// Reads the crate's real `Cargo.toml` and asserts that its workspace-internal
/// regular dependencies stay within the allowed edge table.
///
/// Panics when the manifest cannot be read or parsed, when no regular
/// dependency entry is extracted, when a dev-only crate leaks into a regular
/// section, or when a workspace-internal dependency is outside
/// `allowed_workspace_deps`.
pub fn assert_workspace_dep_boundary(spec: &DepBoundarySpec) {
    let manifest = std::fs::read_to_string(&spec.manifest_path).unwrap_or_else(|error| {
        panic!(
            "{}: cannot read manifest at {}: {error}",
            spec.crate_name,
            spec.manifest_path.display()
        )
    });
    let entries = manifest_dependencies(&manifest).unwrap_or_else(|error| {
        panic!(
            "{}: cannot parse manifest at {}: {error}",
            spec.crate_name,
            spec.manifest_path.display()
        )
    });
    assert!(
        entries.iter().any(|entry| entry.section.is_regular()),
        "{}: parser must extract the [dependencies] entries",
        spec.crate_name
    );
    assert_dev_only_crates_absent(&entries, spec);
    let violations = workspace_internal_violations(&entries, spec.allowed_workspace_deps);
    assert!(
        violations.is_empty(),
        "{}: workspace deps outside the allowed edge table: {violations:?}",
        spec.crate_name
    );
}

fn assert_dev_only_crates_absent(entries: &[DependencyEntry], spec: &DepBoundarySpec) {
    for dev_only in spec.dev_only_workspace_deps {
        let leaked = entries
            .iter()
            .any(|entry| entry.section.is_regular() && &entry.name == dev_only);
        assert!(
            !leaked,
            "{}: dev-only workspace crate {dev_only} must not appear in regular dependencies",
            spec.crate_name
        );
    }
}

fn collect_top_level_sections(parsed: &Value, entries: &mut Vec<DependencyEntry>) {
    collect_section(parsed, "dependencies", DependencySection::Regular, entries);
    collect_section(parsed, "dev-dependencies", DependencySection::Dev, entries);
    collect_section(
        parsed,
        "build-dependencies",
        DependencySection::Build,
        entries,
    );
}

fn collect_target_sections(parsed: &Value, entries: &mut Vec<DependencyEntry>) {
    let Some(targets) = parsed.get("target").and_then(Value::as_table) else {
        return;
    };
    for target in targets.values() {
        collect_section(target, "dependencies", DependencySection::Target, entries);
        collect_section(target, "dev-dependencies", DependencySection::Dev, entries);
        collect_section(
            target,
            "build-dependencies",
            DependencySection::Build,
            entries,
        );
    }
}

fn collect_section(
    table: &Value,
    key: &str,
    section: DependencySection,
    entries: &mut Vec<DependencyEntry>,
) {
    let Some(section_table) = table.get(key).and_then(Value::as_table) else {
        return;
    };
    entries.extend(
        section_table
            .iter()
            .map(|(name, declaration)| DependencyEntry {
                name: name.clone(),
                section,
                declaration: declaration.clone(),
            }),
    );
}
