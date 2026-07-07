//! Plan file creation and management for Plan Mode.
//!
//! Provides functions to generate plan identifiers and create plan files
//! in the `plans/` directory of a workspace.

use chrono::Local;
use std::path::{Path, PathBuf};

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

## Context

## Tasks

## Verification

## Notes

";

/// Generate a plan identifier in `yyyy-MM-dd-HH-mm-ss-{slug}` format.
///
/// The slug is derived from the title by lowercasing and replacing
/// non-alphanumeric characters (except hyphens) with hyphens, then
/// truncating to 50 characters. If the title is empty, a random
/// suffix is used instead.
pub fn generate_identifier(title: &str) -> String {
    let timestamp = Local::now().format("%Y-%m-%d-%H-%M-%S").to_string();

    let slug = if title.is_empty() {
        "untitled".to_string()
    } else {
        slugify(title)
    };

    format!("{timestamp}-{slug}")
}

/// Create a plan file in `{workdir}/plans/` directory.
///
/// The directory is created if it does not exist. The file name is
/// derived from [`generate_identifier`] with a `.md` extension.
/// Returns the path to the created file.
pub fn create_plan_file(workdir: &Path, title: &str) -> Result<PathBuf, std::io::Error> {
    let plans_dir = workdir.join("plans");
    std::fs::create_dir_all(&plans_dir)?;

    let identifier = generate_identifier(title);
    let file_path = plans_dir.join(format!("{identifier}.md"));

    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let content = PLAN_TEMPLATE
        .replace("{title}", title)
        .replace("{timestamp}", &timestamp);

    std::fs::write(&file_path, content)?;

    Ok(file_path)
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
