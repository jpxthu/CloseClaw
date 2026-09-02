//! Unit tests for Step 1.1: prepare_outbound_local_media.
//!
//! Covers:
//! - HTTP URLs return None (already Feishu-hosted)
//! - Absolute path within workspace → resolved
//! - Absolute path within media store → resolved
//! - Absolute path outside allowed dirs → rejected
//! - Relative path resolved against workspace_dir
//! - Relative path with no workspace_dir → rejected
//! - Non-existent file → None

use super::outbound_media::prepare_outbound_local_media;
use crate::media_store::MediaStore;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test MediaStore rooted in a temp directory.
fn make_test_media_store() -> (TempDir, Arc<MediaStore>) {
    let tmp = TempDir::new().expect("tmp dir");
    let store = Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"));
    (tmp, store)
}

// =========================================================================
// HTTP URL tests
// =========================================================================

#[tokio::test]
async fn http_url_returns_none() {
    let (_tmp, store) = make_test_media_store();
    let result = prepare_outbound_local_media("http://example.com/img.png", None, &store).await;
    assert!(result.is_none(), "HTTP URL should return None");
}

#[tokio::test]
async fn https_url_returns_none() {
    let (_tmp, store) = make_test_media_store();
    let result = prepare_outbound_local_media("https://example.com/img.png", None, &store).await;
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

    let (_store_tmp, store) = make_test_media_store();
    let path = ws.join("photo.png");

    let result = prepare_outbound_local_media(path.to_str().unwrap(), Some(&ws), &store).await;
    assert!(
        result.is_some(),
        "absolute path within workspace should resolve"
    );
    // Result should be in the outbound directory, not the original path.
    let outbound = result.unwrap();
    assert!(outbound.exists());
    assert!(outbound.starts_with(store.outbound_dir()));
}

#[tokio::test]
async fn absolute_path_within_media_store_resolves() {
    let tmp = TempDir::new().unwrap();
    let media_dir = tmp.path().join("media");
    std::fs::create_dir_all(&media_dir).unwrap();
    std::fs::write(media_dir.join("doc.pdf"), "pdf data").unwrap();

    let store = Arc::new(MediaStore::new(media_dir.to_str().unwrap()).unwrap());
    let path = media_dir.join("doc.pdf");

    let result = prepare_outbound_local_media(path.to_str().unwrap(), None, &store).await;
    assert!(
        result.is_some(),
        "absolute path within media store should resolve"
    );
    let outbound = result.unwrap();
    assert!(outbound.exists());
    assert!(outbound.starts_with(store.outbound_dir()));
}

#[tokio::test]
async fn absolute_path_outside_rejected() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();

    let (_store_tmp, store) = make_test_media_store();
    let path = outside.join("secret.txt");

    let result = prepare_outbound_local_media(path.to_str().unwrap(), Some(&ws), &store).await;
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

    let (_store_tmp, store) = make_test_media_store();
    let path = ws.join("nonexistent.png");

    let result = prepare_outbound_local_media(path.to_str().unwrap(), Some(&ws), &store).await;
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

    let (_store_tmp, store) = make_test_media_store();

    let result = prepare_outbound_local_media("./file.png", Some(&ws), &store).await;
    assert!(
        result.is_some(),
        "relative path should resolve against workspace"
    );
    let outbound = result.unwrap();
    assert!(outbound.exists());
    assert!(outbound.starts_with(store.outbound_dir()));
}

#[tokio::test]
async fn relative_path_subdirectory_resolves() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(ws.join("subdir")).unwrap();
    std::fs::write(ws.join("subdir").join("nested.pdf"), "data").unwrap();

    let (_store_tmp, store) = make_test_media_store();

    let result = prepare_outbound_local_media("./subdir/nested.pdf", Some(&ws), &store).await;
    assert!(
        result.is_some(),
        "relative path with subdirectory should resolve"
    );
    let outbound = result.unwrap();
    assert!(outbound.exists());
}

#[tokio::test]
async fn relative_path_no_workspace_fails() {
    let (_store_tmp, store) = make_test_media_store();

    // Relative path without workspace_dir — the function returns None
    // because the raw relative path doesn't exist as-is.
    let result = prepare_outbound_local_media("./orphan.png", None, &store).await;
    assert!(
        result.is_none(),
        "relative path without workspace should return None"
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
    std::fs::write(outside.join("escape.txt"), "bad").unwrap();

    let (_store_tmp, store) = make_test_media_store();

    // This relative path resolves to workspace/../outside/escape.txt
    // but canonicalize prevents directory traversal
    let result = prepare_outbound_local_media("../outside/escape.txt", Some(&ws), &store).await;
    assert!(
        result.is_none(),
        "relative path escaping workspace should be rejected"
    );
}
