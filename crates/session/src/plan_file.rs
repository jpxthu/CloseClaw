//! Plan file creation and management for Plan Mode.
//!
//! Provides functions to generate plan identifiers and create plan files
//! in the `plans/` directory of a workspace.

use chrono::Local;
use closeclaw_common::plan_state::PlanStatus;
use closeclaw_config::IdentifierFormat;
use rand::seq::SliceRandom;
use std::path::{Path, PathBuf};

/// Parse `PlanStatus` from a plan file's content.
///
/// Scans the file for the `| 状态 | <status> |` line and converts it
/// to the corresponding [`PlanStatus`] variant.
pub fn parse_plan_status_from_file(content: &str) -> Option<PlanStatus> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("| 状态 | ") {
            let status_str = rest.strip_suffix(" |")?.trim();
            return match status_str {
                "draft" => Some(PlanStatus::Draft),
                "confirmed" => Some(PlanStatus::Confirmed),
                "executing" => Some(PlanStatus::Executing),
                "paused" => Some(PlanStatus::Paused),
                "completed" => Some(PlanStatus::Completed),
                _ => None,
            };
        }
    }
    None
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
/// Contains placeholders for title and timestamp, draft status marker,
/// and skeleton section headers.
pub const PLAN_TEMPLATE: &str = "\
# {title}

| 字段 | 值 |
|------|-----|
| 状态 | draft |
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

/// Update the status field in a plan file.
///
/// Replaces `| 状态 | xxx |` with `| 状态 | {status} |` and also
/// updates the `| 更新时间 | xxx |` field to the current time.
///
/// # Errors
/// Returns an error if the file cannot be read or written, or if
/// the status line is not found.
pub fn update_plan_status(plan_file_path: &str, status: &PlanStatus) -> Result<(), std::io::Error> {
    let path = Path::new(plan_file_path);
    if !path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("plan file not found: {plan_file_path}"),
        ));
    }

    let content = std::fs::read_to_string(path)?;
    let new_timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let status_str = status.to_string();

    let status_replaced = replace_status_line(&content, &status_str).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("status line not found in plan file: {plan_file_path}"),
        )
    })?;

    // Update timestamp if the line exists; skip gracefully if not.
    let new_content =
        replace_update_time_line(&status_replaced, &new_timestamp).unwrap_or(status_replaced);

    std::fs::write(path, new_content)
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

/// Replace the `| 状态 | xxx |` line with the given status.
fn replace_status_line(content: &str, new_status: &str) -> Option<String> {
    let prefix = "| 状态 | ";
    let suffix = " |";
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::with_capacity(lines.len());
    let mut found = false;

    for line in &lines {
        if line.contains("| 状态 | ") && line.ends_with(" |") {
            result.push(format!("{prefix}{new_status}{suffix}"));
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
