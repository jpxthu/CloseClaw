//! Plan file creation and management for Plan Mode.
//!
//! Provides functions to generate plan identifiers and create plan files
//! in the `plans/` directory of a workspace.

use chrono::Local;
use closeclaw_config::IdentifierFormat;
use rand::seq::SliceRandom;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur when resolving a plan file by name.
#[derive(Debug, Error)]
pub enum PlanResolveError {
    /// No plan file matched the given name.
    #[error("plan not found: {name}")]
    NotFound { name: String },

    /// Multiple plan files matched the given name.
    #[error("ambiguous plan name '{name}': {candidates:?}")]
    Ambiguous {
        name: String,
        candidates: Vec<String>,
    },
}

/// Adjective word list for random identifiers (50 words).
const ADJECTIVES: &[&str] = &[
    "calm", "bright", "deep", "swift", "soft", "bold", "clear", "dawn", "fair", "glad", "high",
    "keen", "mild", "neat", "pale", "rich", "safe", "tall", "warm", "wise", "cool", "dark", "fast",
    "gold", "haze", "iron", "jade", "lace", "mint", "noble", "oak", "pure", "rare", "sage", "true",
    "vast", "wild", "zinc", "blue", "clay", "drift", "fern", "glen", "ink", "kite", "lake", "mist",
    "opal", "pine", "reef",
];

/// Noun word list for random identifiers (50 words).
const NOUNS: &[&str] = &[
    "wave", "stone", "river", "flame", "cloud", "field", "forge", "grove", "harbor", "isle",
    "knot", "lance", "moss", "nest", "ocean", "peak", "ridge", "storm", "trail", "vale", "wind",
    "ark", "bell", "cove", "dune", "elm", "frost", "gate", "hill", "jewel", "keel", "lamp",
    "meadow", "oven", "quill", "reed", "star", "tower", "umbra", "vine", "ward", "yew", "zephyr",
    "ash", "bay", "cape", "silk", "tide", "nape", "pine",
];

/// Standard plan file template.
///
/// Contains placeholders for title and timestamp,
/// and skeleton section headers.
pub const PLAN_TEMPLATE: &str = "\
# {title}

| 字段 | 值 |
|------|-----|
| 创建时间 | {timestamp} |
| 更新时间 | {timestamp} |

## Context

## Tasks

## Verification

## Notes

";

/// Generate a plan identifier in `{adjective}-{noun}-{noun}` format.
///
/// Uses `rand` crate for randomness. Words are drawn from built-in
/// adjective and noun lists (50 words each).
pub fn generate_random_identifier() -> String {
    let mut rng = rand::thread_rng();
    let adj = ADJECTIVES
        .choose(&mut rng)
        .expect("ADJECTIVES is non-empty");
    let noun1 = NOUNS.choose(&mut rng).expect("NOUNS is non-empty");
    let noun2 = NOUNS.choose(&mut rng).expect("NOUNS is non-empty");
    format!("{adj}-{noun1}-{noun2}")
}

/// Generate a plan identifier in `yyyy-MM-dd-HH-mm-ss-{slug}` format.
///
/// The slug is derived from the title by lowercasing and replacing
/// non-alphanumeric characters (except hyphens) with hyphens, then
/// truncating to 50 characters. If the title is empty, "untitled"
/// is used instead.
pub fn generate_timestamp_identifier(title: &str) -> String {
    let timestamp = Local::now().format("%Y-%m-%d-%H-%M-%S").to_string();

    let slug = if title.is_empty() {
        "untitled".to_string()
    } else {
        slugify(title)
    };

    format!("{timestamp}-{slug}")
}

/// Generate a plan identifier using the specified format.
///
/// - [`IdentifierFormat::Timestamp`][]: `yyyy-MM-dd-HH-mm-ss-{slug}`
/// - [`IdentifierFormat::RandomWords`][]: `{adjective}-{noun}-{noun}`
pub fn generate_identifier(title: &str, format: IdentifierFormat) -> String {
    match format {
        IdentifierFormat::Timestamp => generate_timestamp_identifier(title),
        IdentifierFormat::RandomWords => generate_random_identifier(),
    }
}

/// Create a plan file in `{workdir}/plans/` directory.
///
/// Uses the default timestamp identifier format. For explicit format
/// control, use [`create_plan_file_with_format`].
pub fn create_plan_file(workdir: &Path, title: &str) -> Result<PathBuf, std::io::Error> {
    create_plan_file_with_format(workdir, title, IdentifierFormat::default())
}

/// Create a plan file with explicit identifier format.
///
/// Like [`create_plan_file`] but allows choosing between timestamp
/// and random-words identifier formats.
pub fn create_plan_file_with_format(
    workdir: &Path,
    title: &str,
    format: IdentifierFormat,
) -> Result<PathBuf, std::io::Error> {
    let plans_dir = workdir.join("plans");
    std::fs::create_dir_all(&plans_dir)?;

    let identifier = generate_identifier(title, format);
    let file_path = plans_dir.join(format!("{identifier}.md"));

    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let content = PLAN_TEMPLATE
        .replace("{title}", title)
        .replace("{timestamp}", &timestamp);

    std::fs::write(&file_path, content)?;

    Ok(file_path)
}

/// Update only the update timestamp field in a plan file.
///
/// Replaces `| 更新时间 | xxx |` with the current time.
///
/// # Errors
/// Returns an error if the file cannot be read or written, or if
/// the update time line is not found.
pub fn update_plan_timestamp(plan_file_path: &str) -> Result<(), std::io::Error> {
    let path = Path::new(plan_file_path);
    if !path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("plan file not found: {plan_file_path}"),
        ));
    }

    let content = std::fs::read_to_string(path)?;
    let new_timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    match replace_update_time_line(&content, &new_timestamp) {
        Some(c) => std::fs::write(path, c),
        None => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("update time line not found in plan file: {plan_file_path}"),
        )),
    }
}

/// Resolve a plan file path by name within a workspace.
///
/// Searches `{workdir}/plans/` for `.md` files and applies the following
/// matching strategy:
///
/// 1. **Exact match** — `{name}.md` (or `{name}` if it already ends with `.md`)
/// 2. **Unique prefix** — exactly one file whose stem starts with `name`
/// 3. **Unique fuzzy** — exactly one file whose stem contains `name`
///
/// Returns the matching [`PathBuf`] on success, or a [`PlanResolveError`]
/// if zero or more than one file matches.
///
/// # Errors
///
/// - [`PlanResolveError::NotFound`] — no file matched
/// - [`PlanResolveError::Ambiguous`] — more than one file matched
pub fn resolve_plan_by_name(workdir: &Path, name: &str) -> Result<PathBuf, PlanResolveError> {
    let plans_dir = workdir.join("plans");
    let files = list_plan_stems(&plans_dir)?;

    let query = strip_md_extension(name);

    if query.is_empty() {
        return Err(PlanResolveError::NotFound {
            name: name.to_string(),
        });
    }

    match_plan_by_stem(&files, &query, name)
}

/// List `.md` file stems in a directory.
///
/// Returns the set of stems (filename without `.md` extension).
/// Returns [`PlanResolveError::NotFound`] if the directory does not
/// exist or contains no `.md` files.
fn list_plan_stems(dir: &Path) -> Result<Vec<String>, PlanResolveError> {
    let entries = std::fs::read_dir(dir).map_err(|_| PlanResolveError::NotFound {
        name: String::new(),
    })?;

    let mut stems = Vec::new();
    for entry in entries.flatten() {
        if let Some(stem) = plan_file_stem(&entry.path()) {
            stems.push(stem);
        }
    }
    Ok(stems)
}

/// Extract the stem from a plan file path, if it is a `.md` file.
fn plan_file_stem(path: &Path) -> Option<String> {
    let ext = path.extension()?;
    if ext != "md" {
        return None;
    }
    path.file_stem()?.to_str().map(|s| s.to_string())
}

/// Strip a trailing `.md` extension from a name, if present.
fn strip_md_extension(name: &str) -> String {
    if let Some(stripped) = name.strip_suffix(".md") {
        stripped.to_string()
    } else {
        name.to_string()
    }
}

/// Match a query against plan file stems using the three-tier strategy.
fn match_plan_by_stem(
    stems: &[String],
    query: &str,
    original_name: &str,
) -> Result<PathBuf, PlanResolveError> {
    // Tier 1: exact match
    let exact: Vec<&str> = stems
        .iter()
        .filter(|s| s.as_str() == query)
        .map(|s| s.as_str())
        .collect();
    if exact.len() == 1 {
        return Ok(plan_file_path(exact[0]));
    }
    if exact.len() > 1 {
        return Err(PlanResolveError::Ambiguous {
            name: original_name.to_string(),
            candidates: exact.into_iter().map(String::from).collect(),
        });
    }

    // Tier 2: prefix match
    let prefix: Vec<&str> = stems
        .iter()
        .filter(|s| s.starts_with(query))
        .map(|s| s.as_str())
        .collect();
    if prefix.len() == 1 {
        return Ok(plan_file_path(prefix[0]));
    }
    if prefix.len() > 1 {
        return Err(PlanResolveError::Ambiguous {
            name: original_name.to_string(),
            candidates: prefix.into_iter().map(String::from).collect(),
        });
    }

    // Tier 3: fuzzy (substring) match
    let fuzzy: Vec<&str> = stems
        .iter()
        .filter(|s| s.contains(query))
        .map(|s| s.as_str())
        .collect();
    if fuzzy.len() == 1 {
        return Ok(plan_file_path(fuzzy[0]));
    }
    if fuzzy.len() > 1 {
        return Err(PlanResolveError::Ambiguous {
            name: original_name.to_string(),
            candidates: fuzzy.into_iter().map(String::from).collect(),
        });
    }

    Err(PlanResolveError::NotFound {
        name: original_name.to_string(),
    })
}

/// Build a plan file path from a stem.
fn plan_file_path(stem: &str) -> PathBuf {
    PathBuf::from(format!("plans/{stem}.md"))
}

/// Replace the `| 更新时间 | xxx |` line with the given timestamp.
fn replace_update_time_line(content: &str, new_timestamp: &str) -> Option<String> {
    let prefix = "| 更新时间 | ";
    let suffix = " |";
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::with_capacity(lines.len());
    let mut found = false;

    for line in &lines {
        if line.contains("| 更新时间 | ") && line.ends_with(" |") {
            result.push(format!("{prefix}{new_timestamp}{suffix}"));
            found = true;
        } else {
            result.push((*line).to_string());
        }
    }

    if found {
        Some(result.join("\n"))
    } else {
        None
    }
}

/// Convert a title string into a URL-friendly slug.
///
/// Rules:
/// - Lowercase all characters
/// - Replace non-alphanumeric characters with hyphens
/// - Collapse consecutive hyphens
/// - Trim leading/trailing hyphens
/// - Truncate to 50 characters
fn slugify(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    // Collapse consecutive hyphens
    let mut result = String::with_capacity(slug.len());
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push(c);
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }

    let trimmed = result.trim_matches('-');

    // Truncate to 50 characters
    let truncated: String = trimmed.chars().take(50).collect();

    // Ensure non-empty
    if truncated.is_empty() {
        "untitled".to_string()
    } else {
        truncated
    }
}

// ── Plan browsing functions ─────────────────────────────────────────────

/// Summary information for a single plan file.
#[derive(Debug, Clone)]
pub struct PlanSummary {
    /// File stem (filename without `.md` extension).
    pub stem: String,
    /// Plan title extracted from the first heading line.
    pub title: String,
    /// Number of tasks in a terminal state (`[x]`, `[!]`, `[~]`).
    pub completed: usize,
    /// Total number of tasks (all checkbox lines in Tasks section).
    pub total: usize,
}

/// List all plan summaries in `{workdir}/plans/`.
///
/// Scans for `.md` files, parses each plan's title and task
/// completion counts, and returns results sorted by modification
/// time (most recent first). If the plans directory does not
/// exist, returns an empty vector.
pub fn list_plan_summaries(workdir: &Path) -> io::Result<Vec<PlanSummary>> {
    let plans_dir = workdir.join("plans");
    if !plans_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<_> = std::fs::read_dir(&plans_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| plan_file_stem(&e.path()).is_some())
        .collect();
    entries.sort_by(|a, b| {
        let time_a = a.metadata().and_then(|m| m.modified()).ok();
        let time_b = b.metadata().and_then(|m| m.modified()).ok();
        time_b.cmp(&time_a)
    });
    let mut summaries = Vec::new();
    for entry in entries {
        let path = entry.path();
        let stem = plan_file_stem(&path).unwrap_or_default();
        let content = std::fs::read_to_string(&path)?;
        let title = extract_title(&content);
        let (completed, total) = count_tasks(&content);
        summaries.push(PlanSummary {
            stem,
            title,
            completed,
            total,
        });
    }
    Ok(summaries)
}

/// Read the full content of a plan file at the given path.
pub fn read_plan_content(path: &Path) -> io::Result<String> {
    std::fs::read_to_string(path)
}

/// Extract the title from the first `# ` heading line.
fn extract_title(content: &str) -> String {
    content
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(|t| t.trim().to_string()))
        .unwrap_or_default()
}

/// Count completed and total tasks in the Tasks section.
///
/// Completed: lines matching `[x]`, `[!]`, or `[~]`.
/// Total: all lines in the Tasks section starting with `- [`.
fn count_tasks(content: &str) -> (usize, usize) {
    let mut in_tasks = false;
    let mut completed = 0usize;
    let mut total = 0usize;
    for line in content.lines() {
        if line.trim().starts_with("## Tasks") {
            in_tasks = true;
            continue;
        }
        if in_tasks && line.trim().starts_with("## ") {
            break;
        }
        if !in_tasks {
            continue;
        }
        let trimmed = line.trim();
        if !trimmed.starts_with("- [") {
            continue;
        }
        total += 1;
        if trimmed.starts_with("- [x]")
            || trimmed.starts_with("- [!]")
            || trimmed.starts_with("- [~]")
        {
            completed += 1;
        }
    }
    (completed, total)
}
