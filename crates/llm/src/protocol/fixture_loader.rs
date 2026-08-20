//! Protocol fixture loader — test-only helper for loading shared fixture data.
//!
//! Scans `tests/fixtures/fake_llm/openai/` and `anthropic/` directories
//! from the workspace root, loading JSON protocol fixtures and raw SSE
//! streaming text files. This module mirrors `closeclaw_fake_llm::fixture_loader`
//! but lives in the `llm` crate to avoid cross-crate test dependencies.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// A loaded protocol fixture entry, pairing file metadata with parsed content.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ProtocolEntry {
    /// Display name derived from the file stem (e.g., `"simple"`, `"streaming"`).
    pub name: String,
    /// Absolute path to the fixture file.
    pub path: PathBuf,
    /// Parsed fixture content.
    pub fixture: ProtocolFixture,
}

/// Parsed content of a protocol fixture JSON file.
///
/// Fields like `response`, `request`, and `tools_sent` are optional because
/// streaming `.json` meta files omit `response` and may omit `tools_sent`.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ProtocolFixture {
    /// Protocol identifier (`"openai"` or `"anthropic"`).
    pub protocol: String,
    /// Whether this is a streaming fixture.
    pub streaming: bool,
    /// Scenario name (e.g., `"simple"`, `"reasoning"`).
    pub scenario: String,
    /// Model identifier used in the fixture.
    pub model: String,
    /// Expected response shape hint (e.g., `"text"`, `"reasoning"`).
    pub expect: String,
    /// The request payload sent to the LLM (optional for streaming meta).
    #[serde(default)]
    pub request: Option<serde_json::Value>,
    /// The expected response payload (absent for streaming fixtures).
    #[serde(default)]
    pub response: Option<serde_json::Value>,
    /// Tool definitions sent in the request (optional).
    #[serde(default)]
    pub tools_sent: Option<Vec<serde_json::Value>>,
    /// Maximum tokens sent (Anthropic fixtures).
    #[serde(default)]
    pub max_tokens_sent: Option<u32>,
}

/// Resolve the workspace fixture root directory.
///
/// The fixture tree lives at `tests/fixtures/fake_llm/` relative to the
/// workspace root, which is two levels up from `CARGO_MANIFEST_DIR`.
pub fn fixture_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("fake_llm")
}

/// Load a single protocol fixture JSON file.
///
/// Returns an error with the file path in the message on read/parse failure.
pub fn load_protocol_fixture(path: &Path) -> Result<ProtocolFixture> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read fixture file: {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse fixture file: {}", path.display()))
}

/// Load a streaming SSE text file as a raw string.
pub fn load_streaming_fixture(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("failed to read streaming fixture: {}", path.display()))
}

/// Load a streaming meta JSON file as an arbitrary JSON value.
pub fn load_streaming_meta(path: &Path) -> Result<serde_json::Value> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read streaming meta: {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse streaming meta: {}", path.display()))
}

/// Load all protocol fixture JSON files from a directory.
///
/// Only `.json` files are loaded; other extensions are skipped.
/// Returns entries sorted by file name for deterministic ordering.
pub fn load_protocol_fixtures_dir(dir: &Path) -> Result<Vec<ProtocolEntry>> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read fixture directory: {}", dir.display()))?;

    let mut result: Vec<ProtocolEntry> = Vec::new();

    for entry in entries {
        let entry = entry
            .with_context(|| format!("failed to read directory entry in: {}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let fixture = load_protocol_fixture(&path)?;
        result.push(ProtocolEntry {
            name,
            path,
            fixture,
        });
    }

    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

/// Load all streaming SSE text files from a directory.
///
/// Returns `(name, raw_content)` pairs for `.txt` files, sorted by name.
pub fn load_streaming_fixtures_dir(dir: &Path) -> Result<Vec<(String, String)>> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read fixture directory: {}", dir.display()))?;

    let mut result: Vec<(String, String)> = Vec::new();

    for entry in entries {
        let entry = entry
            .with_context(|| format!("failed to read directory entry in: {}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let content = load_streaming_fixture(&path)?;
        result.push((name, content));
    }

    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

/// Load all streaming meta JSON files from a directory.
///
/// Returns `(name, meta_value)` pairs for `-meta.json` files, sorted by name.
pub fn load_streaming_metas_dir(dir: &Path) -> Result<Vec<(String, serde_json::Value)>> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read fixture directory: {}", dir.display()))?;

    let mut result: Vec<(String, serde_json::Value)> = Vec::new();

    for entry in entries {
        let entry = entry
            .with_context(|| format!("failed to read directory entry in: {}", dir.display()))?;
        let path = entry.path();
        let name_str = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name_str.ends_with("-meta.json") {
            continue;
        }
        let stem = name_str
            .strip_suffix("-meta.json")
            .unwrap_or(name_str)
            .to_string();
        let meta = load_streaming_meta(&path)?;
        result.push((stem, meta));
    }

    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

/// Resolve the OpenAI fixture directory.
pub fn openai_fixture_dir() -> PathBuf {
    fixture_root().join("openai")
}

/// Resolve the Anthropic fixture directory.
pub fn anthropic_fixture_dir() -> PathBuf {
    fixture_root().join("anthropic")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_root_exists() {
        let root = fixture_root();
        assert!(
            root.is_dir(),
            "fixture root should exist: {}",
            root.display()
        );
    }

    #[test]
    fn openai_fixture_dir_exists() {
        let dir = openai_fixture_dir();
        assert!(
            dir.is_dir(),
            "openai fixture dir should exist: {}",
            dir.display()
        );
    }

    #[test]
    fn anthropic_fixture_dir_exists() {
        let dir = anthropic_fixture_dir();
        assert!(
            dir.is_dir(),
            "anthropic fixture dir should exist: {}",
            dir.display()
        );
    }

    #[test]
    fn openai_load_all_protocol_fixtures() {
        let entries = load_protocol_fixtures_dir(&openai_fixture_dir()).unwrap();
        // JSON files: simple, reasoning, tool-use, cache, error-auth,
        // error-rate-limit, error-server, streaming-meta,
        // tool-use-streaming-meta
        assert_eq!(entries.len(), 9, "expected 9 OpenAI JSON fixtures");
        for e in &entries {
            assert_eq!(e.fixture.protocol, "openai");
        }
    }

    #[test]
    fn anthropic_load_all_protocol_fixtures() {
        let entries = load_protocol_fixtures_dir(&anthropic_fixture_dir()).unwrap();
        // JSON files: anthropic-simple, anthropic-thinking, anthropic-tool-use,
        // anthropic-cache, anthropic-error, anthropic-streaming-meta,
        // anthropic-tool-use-streaming-meta
        assert_eq!(entries.len(), 7, "expected 7 Anthropic JSON fixtures");
        for e in &entries {
            assert_eq!(e.fixture.protocol, "anthropic");
        }
    }

    #[test]
    fn openai_load_all_streaming_fixtures() {
        let files = load_streaming_fixtures_dir(&openai_fixture_dir()).unwrap();
        // 2 txt files: streaming.txt, tool-use-streaming.txt
        assert_eq!(files.len(), 2, "expected 2 OpenAI streaming txt files");
        for (name, content) in &files {
            assert!(
                !content.is_empty(),
                "streaming fixture '{}' should not be empty",
                name
            );
        }
    }

    #[test]
    fn anthropic_load_all_streaming_fixtures() {
        let files = load_streaming_fixtures_dir(&anthropic_fixture_dir()).unwrap();
        assert_eq!(files.len(), 2, "expected 2 Anthropic streaming txt files");
        for (name, content) in &files {
            assert!(
                !content.is_empty(),
                "streaming fixture '{}' should not be empty",
                name
            );
        }
    }

    #[test]
    fn openai_load_all_streaming_metas() {
        let metas = load_streaming_metas_dir(&openai_fixture_dir()).unwrap();
        assert_eq!(metas.len(), 2, "expected 2 OpenAI streaming meta files");
    }

    #[test]
    fn anthropic_load_all_streaming_metas() {
        let metas = load_streaming_metas_dir(&anthropic_fixture_dir()).unwrap();
        assert_eq!(metas.len(), 2, "expected 2 Anthropic streaming meta files");
    }

    #[test]
    fn load_protocol_fixture_simple_openai() {
        let path = openai_fixture_dir().join("simple.json");
        let fixture = load_protocol_fixture(&path).unwrap();
        assert_eq!(fixture.protocol, "openai");
        assert!(!fixture.streaming);
        assert_eq!(fixture.scenario, "simple");
        assert!(fixture.response.is_some());
    }

    #[test]
    fn load_protocol_fixture_simple_anthropic() {
        let path = anthropic_fixture_dir().join("anthropic-simple.json");
        let fixture = load_protocol_fixture(&path).unwrap();
        assert_eq!(fixture.protocol, "anthropic");
        assert!(!fixture.streaming);
        assert_eq!(fixture.scenario, "anthropic-simple");
        assert!(fixture.response.is_some());
    }

    #[test]
    fn load_streaming_fixture_txt() {
        let path = openai_fixture_dir().join("streaming.txt");
        let content = load_streaming_fixture(&path).unwrap();
        assert!(
            content.contains("data:"),
            "SSE text should contain data: lines"
        );
    }

    #[test]
    fn load_protocol_fixture_error_includes_path() {
        let result = load_protocol_fixture(Path::new("/nonexistent/file.json"));
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("/nonexistent/file.json"),
            "error should include file path: {}",
            err_msg
        );
    }

    #[test]
    fn load_streaming_fixture_error_includes_path() {
        let result = load_streaming_fixture(Path::new("/nonexistent/file.txt"));
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("/nonexistent/file.txt"),
            "error should include file path: {}",
            err_msg
        );
    }
}
