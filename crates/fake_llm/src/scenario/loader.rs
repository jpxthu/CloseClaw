//! Scenario file loader.
//!
//! Reads scenario JSON files from disk and deserializes them into
//! [`ScenarioFile`] structs. Supports loading individual files or
//! scanning entire directories.

use std::path::Path;

use anyhow::{Context, Result};

use super::types::ScenarioFile;

/// Load a single scenario file from the given path.
///
/// The file must contain valid JSON that deserializes into a [`ScenarioFile`].
/// Returns an error with context if the file cannot be read or parsed.
pub fn load_scenario_file(path: &Path) -> Result<ScenarioFile> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read scenario file: {}", path.display()))?;

    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse scenario file: {}", path.display()))
}

/// Load all `.json` scenario files in the given directory.
///
/// Files that fail to load are reported as errors. The directory itself
/// must exist and be readable.
pub fn load_scenario_dir(dir: &Path) -> Result<Vec<ScenarioFile>> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read scenario directory: {}", dir.display()))?;

    let mut files = Vec::new();

    for entry in entries {
        let entry = entry
            .with_context(|| format!("failed to read directory entry in: {}", dir.display()))?;

        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let file = load_scenario_file(&path)?;
        files.push(file);
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_temp_scenario(dir: &Path, name: &str, json: &str) {
        fs::write(dir.join(name), json).unwrap();
    }

    #[test]
    fn load_single_file_ok() {
        let tmp = TempDir::new().unwrap();
        let json = r#"{
            "scenarios": [
                {
                    "name": "basic",
                    "turns": [
                        {
                            "response": {
                                "type": "text",
                                "content": "Hello!"
                            }
                        }
                    ]
                }
            ]
        }"#;
        make_temp_scenario(tmp.path(), "test.json", json);

        let file = load_scenario_file(&tmp.path().join("test.json")).unwrap();
        assert_eq!(file.scenarios.len(), 1);
        assert_eq!(file.scenarios[0].name, "basic");
    }

    #[test]
    fn load_file_not_found() {
        let result = load_scenario_file(Path::new("/nonexistent/path/file.json"));
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("failed to read scenario file"));
    }

    #[test]
    fn load_file_invalid_json() {
        let tmp = TempDir::new().unwrap();
        make_temp_scenario(tmp.path(), "bad.json", "{not valid json");

        let result = load_scenario_file(&tmp.path().join("bad.json"));
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("failed to parse scenario file"));
    }

    #[test]
    fn load_directory_ok() {
        let tmp = TempDir::new().unwrap();

        let json1 = r#"{"scenarios": [{"name": "a", "turns": [{"response": {"type": "text", "content": "A"}}]}]}"#;
        let json2 = r#"{"scenarios": [{"name": "b", "turns": [{"response": {"type": "text", "content": "B"}}]}]}"#;
        make_temp_scenario(tmp.path(), "first.json", json1);
        make_temp_scenario(tmp.path(), "second.json", json2);

        let files = load_scenario_dir(tmp.path()).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn load_directory_skips_non_json() {
        let tmp = TempDir::new().unwrap();
        let json = r#"{"scenarios": [{"name": "ok", "turns": [{"response": {"type": "text", "content": "ok"}}]}]}"#;
        make_temp_scenario(tmp.path(), "good.json", json);
        make_temp_scenario(tmp.path(), "readme.txt", "not json");
        make_temp_scenario(tmp.path(), "config.yaml", "not json either");

        let files = load_scenario_dir(tmp.path()).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn load_directory_not_found() {
        let result = load_scenario_dir(Path::new("/nonexistent/dir"));
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("failed to read scenario directory"));
    }

    #[test]
    fn load_directory_propagates_parse_error() {
        let tmp = TempDir::new().unwrap();
        make_temp_scenario(tmp.path(), "bad.json", "{invalid");

        let result = load_scenario_dir(tmp.path());
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("failed to parse scenario file"));
    }
}
