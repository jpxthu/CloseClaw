//! Unit tests for Step 1.1: try_resolve_media_path.
//!
//! Covers:
//! - HTTP URLs return None (already Feishu-hosted)
//! - Absolute path within workspace → resolved
//! - Absolute path within media store → resolved
//! - Absolute path outside allowed dirs → rejected
//! - Relative path resolved against workspace_dir
//! - Relative path with no workspace_dir → rejected
//! - Non-existent file → None

use super::*;
use crate::media_store::MediaStore;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test MediaStore rooted in a temp directory.
fn make_test_media_store() -> Arc<MediaStore> {
    let tmp = TempDir::new().expect("tmp dir");
    Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"))
}

/// Create a FeishuPlugin with a workspace_dir for testing.
fn make_plugin_with_workspace(workspace: &std::path::Path) -> FeishuPlugin {
    let adapter = Arc::new(
        FeishuAdapter::new("test_profile".into(), make_test_media_store())
        .with_workspace_dir(Some(workspace.to_path_buf())),
    );
    FeishuPlugin::new(adapter)
}

/// Create a FeishuPlugin without a workspace_dir.
fn make_plugin_no_workspace() -> FeishuPlugin {
    let adapter = Arc::new(FeishuAdapter::new("test_profile".into(), make_test_media_store()));
    FeishuPlugin::new(adapter)
}

// =========================================================================
// HTTP URL tests
// =========================================================================

#[tokio::test]
async fn http_url_returns_none() {
    let plugin = make_plugin_no_workspace();
    let store = make_test_media_store();
    let result = plugin
        .try_resolve_media_path("http://example.com/img.png", &store)
        .await;
    assert!(result.is_none(), "HTTP URL should return None");
}

#[tokio::test]
async fn https_url_returns_none() {
    let plugin = make_plugin_no_workspace();
    let store = make_test_media_store();
    let result = plugin
        .try_resolve_media_path("https://example.com/img.png", &store)
        .await;
    assert!(result.is_none(), "HTTPS URL should return None");
}

// =========================================================================
// Absolute path tests
// =========================================================================

#[tokio::test]
async fn absolute_path_within_workspace_resolves() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("photo.png"), "data").unwrap();

    let plugin = make_plugin_with_workspace(&ws);
    let store = make_test_media_store();
    let path = ws.join("photo.png");

    let result = plugin
        .try_resolve_media_path(path.to_str().unwrap(), &store)
        .await;
    assert!(
        result.is_some(),
        "absolute path within workspace should resolve"
    );
    assert_eq!(result.unwrap(), path);
}

#[tokio::test]
async fn absolute_path_within_media_store_resolves() {
    let tmp = TempDir::new().unwrap();
    let media_dir = tmp.path().join("media");
    std::fs::create_dir_all(&media_dir).unwrap();
    std::fs::write(media_dir.join("doc.pdf"), "pdf data").unwrap();

    // Use the adapter's own media_store so its storage_dir matches.
    let media_store = Arc::new(MediaStore::new(media_dir.to_str().unwrap()).unwrap());
    let adapter = Arc::new(FeishuAdapter::new("test_profile".into(), media_store.clone()));
    let plugin = FeishuPlugin::new(adapter);
    let path = media_dir.join("doc.pdf");

    let result = plugin
        .try_resolve_media_path(path.to_str().unwrap(), &media_store)
        .await;
    assert!(
        result.is_some(),
        "absolute path within media store should resolve"
    );
    assert_eq!(result.unwrap(), path);
}

#[tokio::test]
async fn absolute_path_outside_rejected() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();

    let plugin = make_plugin_with_workspace(&ws);
    let store = make_test_media_store();
    let path = outside.join("secret.txt");

    let result = plugin
        .try_resolve_media_path(path.to_str().unwrap(), &store)
        .await;
    assert!(
        result.is_none(),
        "path outside workspace and media store should be rejected"
    );
}

#[tokio::test]
async fn nonexistent_file_returns_none() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();

    let plugin = make_plugin_with_workspace(&ws);
    let store = make_test_media_store();
    let path = ws.join("nonexistent.png");

    let result = plugin
        .try_resolve_media_path(path.to_str().unwrap(), &store)
        .await;
    assert!(result.is_none(), "non-existent file should return None");
}

// =========================================================================
// Relative path tests
// =========================================================================

#[tokio::test]
async fn relative_path_resolved_against_workspace() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("file.png"), "image data").unwrap();

    let plugin = make_plugin_with_workspace(&ws);
    let store = make_test_media_store();

    let result = plugin.try_resolve_media_path("./file.png", &store).await;
    assert!(
        result.is_some(),
        "relative path should resolve against workspace"
    );
    assert_eq!(result.unwrap(), ws.join("file.png"));
}

#[tokio::test]
async fn relative_path_subdirectory_resolves() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(ws.join("subdir")).unwrap();
    std::fs::write(ws.join("subdir").join("nested.pdf"), "data").unwrap();

    let plugin = make_plugin_with_workspace(&ws);
    let store = make_test_media_store();

    let result = plugin
        .try_resolve_media_path("./subdir/nested.pdf", &store)
        .await;
    assert!(
        result.is_some(),
        "relative path with subdirectory should resolve"
    );
    assert_eq!(result.unwrap(), ws.join("subdir/nested.pdf"));
}

#[tokio::test]
async fn relative_path_no_workspace_fails() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("orphan.png"), "data").unwrap();

    let plugin = make_plugin_no_workspace();
    let store = make_test_media_store();

    // Relative path without workspace_dir — canonicalize will fail
    // because the path doesn't exist relative to CWD.
    let result = plugin.try_resolve_media_path("./orphan.png", &store).await;
    assert!(
        result.is_none(),
        "relative path without workspace should fail validation"
    );
}

#[tokio::test]
async fn relative_path_outside_workspace_rejected() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    // Create a file outside workspace that the relative path would resolve to.
    // Since the relative path is resolved against workspace, it won't find
    // the file there, so it returns None.
    std::fs::write(outside.join("escape.txt"), "bad").unwrap();

    let plugin = make_plugin_with_workspace(&ws);
    let store = make_test_media_store();

    // This relative path resolves to workspace/../../outside/escape.txt
    // but canonicalize prevents directory traversal
    let result = plugin
        .try_resolve_media_path("../outside/escape.txt", &store)
        .await;
    assert!(
        result.is_none(),
        "relative path escaping workspace should be rejected"
    );
}
