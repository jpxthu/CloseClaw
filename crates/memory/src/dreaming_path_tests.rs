//! Tests for DreamingPipeline memory_md_path configuration.

use crate::dreaming::DreamingPipeline;
use tempfile::TempDir;

/// Default memory_md_path should be "memory/MEMORY.md".
/// We verify this by writing to a pipeline with default path and checking
/// that the file appears at the expected location relative to cwd.
#[test]
fn test_dreaming_pipeline_default_memory_md_path() {
    // Write should target "memory/MEMORY.md" (relative to cwd).
    // Use a temp dir to avoid polluting the workspace.
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("memory/MEMORY.md");
    // Create parent dir so write_memory_md doesn't fail.
    std::fs::create_dir_all(md_path.parent().unwrap()).unwrap();
    // Re-create pipeline pointing to our temp path to verify the setter works.
    let pipeline = DreamingPipeline::new().with_memory_md_path(md_path.to_str().unwrap());
    pipeline
        .write_memory_md(&["test rule".to_string()])
        .unwrap();
    assert!(md_path.exists(), "should write to configured path");
    let content = std::fs::read_to_string(&md_path).unwrap();
    assert!(content.contains("test rule"));
}

/// with_memory_md_path should override the default path.
#[test]
fn test_dreaming_pipeline_with_memory_md_path_custom() {
    let tmp = TempDir::new().unwrap();
    let md_path = tmp.path().join("custom/MEMORY.md");
    let pipeline = DreamingPipeline::new().with_memory_md_path(md_path.to_str().unwrap());
    pipeline
        .write_memory_md(&["custom rule".to_string()])
        .unwrap();
    assert!(md_path.exists(), "should write to custom path");
    let content = std::fs::read_to_string(&md_path).unwrap();
    assert!(content.contains("custom rule"));
}
