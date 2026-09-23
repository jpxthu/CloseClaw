//! Plan file creation and management for Plan Mode.
//!
//! Provides functions to generate plan identifiers and create plan files
//! in the `plans/` directory of a workspace.

use chrono::{DateTime, Local, Utc};
use closeclaw_config::{write_atomically, IdentifierFormat};
use rand::seq::SliceRandom;
use std::fs::File;
use std::io;
use std::os::unix::fs::PermissionsExt;
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

// ── Concurrency-safe read-modify-write ─────────────────────────────────

/// Perform a read-modify-write cycle on a plan file under an exclusive lock.
///
/// The whole cycle — read → `transform` → atomic write-back — runs while
/// holding an exclusive advisory lock on the sidecar `{path}.lock` file, so
/// concurrent writers (threads and separate processes alike) are serialized
/// and can never observe a truncated file or lose each other's updates. The
/// write-back goes through [`closeclaw_config::write_atomically`] (tempfile +
/// fsync + rename) with the original file mode preserved, so readers only
/// ever see the complete old content or the complete new content.
///
/// The sidecar's `lock` extension never matches the `.md` filter used by plan
/// resolution, listing, and archiving, so lock files stay invisible to them.
/// No global mutable state is involved: exclusion is scoped to the sidecar
/// file only.
///
/// # Errors
/// Returns `NotFound` if the plan file does not exist, otherwise the first
/// error from creating/locking the sidecar, reading the plan file, running
/// `transform`, or writing the result back.
fn mutate_plan_file<F>(plan_path: &Path, transform: F) -> io::Result<()>
where
    F: FnOnce(&mut String) -> io::Result<()>,
{
    if !plan_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("plan file not found: {}", plan_path.display()),
        ));
    }

    let _lock = acquire_plan_lock(plan_path)?;

    let mut content = std::fs::read_to_string(plan_path)?;
    transform(&mut content)?;

    let mode = std::fs::metadata(plan_path)?.permissions().mode();
    write_atomically(plan_path, content.as_bytes(), Some(mode))
}

/// Open (creating it if needed) the sidecar lock file for `plan_path` and
/// take the exclusive advisory lock on it.
///
/// The sidecar lives at `{plan_path}.lock`; the caller holds the lock until
/// the returned [`File`] is dropped. Its `lock` extension never matches the
/// `.md` filter used by plan resolution, listing, and archiving, so lock
/// files stay invisible to them.
///
/// This is the single lock domain shared by every plan-file writer — the
/// [`mutate_plan_file`] read-modify-write helpers and the archival rename in
/// `plan_archive` alike. It involves no global mutable state.
///
/// # Errors
/// Returns an error if the sidecar cannot be created or locked.
pub(crate) fn acquire_plan_lock(plan_path: &Path) -> io::Result<File> {
    let mut lock_name = plan_path.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock_file = File::create(PathBuf::from(&lock_name))?;
    lock_file.lock()?;
    Ok(lock_file)
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
    mutate_plan_file(path, |content| refresh_update_time_line(content, path))
}

/// Rewrite the `| 更新时间 | … |` line in `content` to the current local time.
///
/// Called from within a [`mutate_plan_file`] transform, so the timestamp is
/// taken while the sidecar lock is held and the refresh lands in the same
/// atomic write-back as the surrounding modification.
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidData`] when the update-time line is missing.
fn refresh_update_time_line(content: &mut String, path: &Path) -> io::Result<()> {
    let new_timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let updated = replace_update_time_line(content, &new_timestamp).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "update time line not found in plan file: {}",
                path.display()
            ),
        )
    })?;
    *content = updated;
    Ok(())
}

// ── Application-layer access timestamp ───────────────────────────────────

/// HTML comment marker prefix for the access timestamp in plan files.
///
/// Stored as `<!-- accessed: {ISO-8601 UTC} -->` on the line immediately
/// after the `# {title}` heading.  The marker is portable: it travels with
/// the file across renames (archive) and requires no external storage.
const ACCESS_TIMESTAMP_MARKER_PREFIX: &str = "<!-- accessed: ";
const ACCESS_TIMESTAMP_MARKER_SUFFIX: &str = " -->";

/// Read the application-layer access timestamp from a plan file.
///
/// Looks for an `<!-- accessed: {ISO-8601 UTC} -->` marker after the
/// title heading.  Returns `None` if the file has no marker (legacy /
/// newly-created plans).
///
/// # Errors
/// Returns an error if the file cannot be read.
pub fn read_access_timestamp(plan_path: &Path) -> Result<Option<DateTime<Utc>>, std::io::Error> {
    let content = std::fs::read_to_string(plan_path)?;
    Ok(parse_access_timestamp(&content))
}

/// Update (or insert) the access timestamp in a plan file.
///
/// If the marker already exists, its value is replaced.  If it does not
/// exist, a new marker is inserted on the line immediately after the
/// title heading (`# …`).  Plan files without a title heading are
/// rejected with an error.
///
/// # Errors
/// Returns an error if the file cannot be read/written, or if the
/// title heading line is missing.
pub fn touch_access_timestamp(plan_path: &Path) -> Result<(), std::io::Error> {
    mutate_plan_file(plan_path, |content| {
        // `now` is taken inside the lock so serialized writers apply markers
        // in lock-acquisition order and the timestamp never regresses.
        let marker = format!("<!-- accessed: {} -->", Utc::now().to_rfc3339());
        apply_access_marker(content, &marker)
    })
}

/// Replace the existing access timestamp marker in `content` with `marker`,
/// or insert `marker` on the line immediately after the `# {title}` heading.
///
/// Files with an unterminated marker or without a title heading are rejected
/// with [`io::ErrorKind::InvalidData`].
fn apply_access_marker(content: &mut String, marker: &str) -> io::Result<()> {
    if let Some(idx) = content.find(ACCESS_TIMESTAMP_MARKER_PREFIX) {
        let start = idx;
        let end = match content[start..].find(ACCESS_TIMESTAMP_MARKER_SUFFIX) {
            Some(e) => start + e + ACCESS_TIMESTAMP_MARKER_SUFFIX.len(),
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "malformed access timestamp marker in plan file",
                ));
            }
        };
        content.replace_range(start..end, marker);
    } else {
        // Insert after the `# {title}` heading line.
        // The heading may be at the very start of the file (no leading newline)
        // or preceded by a newline.
        let insert_pos = content
            .find("\n# ")
            .map(|p| p + 1) // after the first newline, before "# "
            .or_else(|| {
                // Title heading at the very start of the file
                if content.starts_with("# ") {
                    content.find('\n').map(|p| p + 1) // right after the title line's newline
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "cannot insert access timestamp: plan file must start with a '# Title' heading",
                )
            })?;
        content.insert_str(insert_pos, &format!("{marker}\n"));
    }
    Ok(())
}

/// Parse the access timestamp marker from plan file content.
fn parse_access_timestamp(content: &str) -> Option<DateTime<Utc>> {
    let idx = content.find(ACCESS_TIMESTAMP_MARKER_PREFIX)?;
    let start = idx + ACCESS_TIMESTAMP_MARKER_PREFIX.len();
    let rest = &content[start..];
    let end = rest.find(ACCESS_TIMESTAMP_MARKER_SUFFIX)?;
    let ts_str = &rest[..end];
    DateTime::parse_from_rfc3339(ts_str)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
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

// ── Incremental write API ───────────────────────────────────────────────

/// Append content to a named section in a plan file.
///
/// The section is identified by the heading `## {section}`. Content is
/// inserted after the section heading and before the next `##` heading
/// (or end of file). If the section does not exist, a new section is
/// appended at the end of the file.
///
/// After writing, the update timestamp is automatically refreshed.
///
/// # Errors
/// Returns an error if the file cannot be read or written, or if the
/// timestamp update fails.
pub fn append_to_plan_section(
    plan_path: &Path,
    section: &str,
    content: &str,
) -> Result<(), std::io::Error> {
    let section_heading = format!("## {section}");
    mutate_plan_file(plan_path, |file_content| {
        if let Some(insert_pos) = find_section_insert_position(file_content, &section_heading) {
            // Insert content before the next ## heading (or at end)
            file_content.insert_str(insert_pos, content);
        } else {
            // Section doesn't exist; append at end
            if !file_content.ends_with('\n') {
                file_content.push('\n');
            }
            file_content.push_str(&format!("\n{section_heading}\n\n{content}"));
        }
        // Refresh update timestamp within the same locked, atomic write-back
        refresh_update_time_line(file_content, plan_path)
    })
}

/// Read the content of a named section in a plan file.
///
/// Returns the text between `## {section}` and the next `##` heading
/// (or end of file), excluding the heading line itself. Returns an
/// empty string if the section does not exist.
///
/// # Errors
/// Returns an error if the file cannot be read.
pub fn read_plan_section(plan_path: &Path, section: &str) -> Result<String, std::io::Error> {
    let content = std::fs::read_to_string(plan_path)?;
    let section_heading = format!("## {section}");

    Ok(extract_section_content(&content, &section_heading))
}

/// Find the byte offset where new content should be inserted for a
/// section. Returns the position after the section heading line and
/// its trailing blank line, before the next `##` heading.
///
/// Returns `None` if the section heading is not found.
fn find_section_insert_position(content: &str, section_heading: &str) -> Option<usize> {
    let lines: Vec<&str> = content.lines().collect();
    let mut in_section = false;
    let mut byte_pos = 0usize;
    let mut last_content_end = 0usize; // byte_pos after last non-empty content line

    for line in &lines {
        let line_len = line.len();
        if line.trim() == section_heading {
            in_section = true;
            // Position after heading line + trailing newline
            byte_pos += line_len + 1;
            // Skip blank line after heading if present
            if byte_pos < content.len() && content.as_bytes().get(byte_pos) == Some(&b'\n') {
                byte_pos += 1;
            }
            last_content_end = byte_pos;
            continue;
        }
        if in_section && line.trim().starts_with("## ") {
            return Some(last_content_end);
        }
        byte_pos += line_len + 1; // +1 for newline
        if in_section && !line.is_empty() {
            last_content_end = byte_pos;
        }
    }
    if in_section {
        Some(last_content_end)
    } else {
        None
    }
}

/// Extract the text content of a section, excluding the heading.
fn extract_section_content(content: &str, section_heading: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut in_section = false;
    let mut section_lines = Vec::new();

    for line in &lines {
        if line.trim() == section_heading {
            in_section = true;
            continue;
        }
        if in_section && line.trim().starts_with("## ") {
            break;
        }
        if in_section {
            section_lines.push(*line);
        }
    }

    // Trim leading/trailing blank lines from extracted content
    let trimmed = section_lines.join("\n");
    trimmed.trim().to_string()
}

// ── Plan browsing functions ─────────────────────────────────────────────

/// Summary information for a single plan file.
#[derive(Debug, Clone)]
pub struct PlanSummary {
    /// File stem (filename without `.md` extension).
    pub stem: String,
    /// Plan title extracted from the first heading line.
    pub title: String,
    /// Number of tasks completed (`[x]`).
    pub completed: usize,
    /// Number of tasks failed (`[!]`).
    pub failed: usize,
    /// Number of tasks skipped (`[~]`).
    pub skipped: usize,
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
        let (completed, failed, skipped, total) = count_tasks(&content);
        summaries.push(PlanSummary {
            stem,
            title,
            completed,
            failed,
            skipped,
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

/// Count completed, failed, skipped, and total tasks in the Tasks section.
///
/// Completed: lines matching `[x]`. Failed: `[!]`. Skipped: `[~]`.
/// Total: all lines in the Tasks section starting with `- [`.
fn count_tasks(content: &str) -> (usize, usize, usize, usize) {
    let mut in_tasks = false;
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
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
        if trimmed.starts_with("- [x]") {
            completed += 1;
        } else if trimmed.starts_with("- [!]") {
            failed += 1;
        } else if trimmed.starts_with("- [~]") {
            skipped += 1;
        }
    }
    (completed, failed, skipped, total)
}
