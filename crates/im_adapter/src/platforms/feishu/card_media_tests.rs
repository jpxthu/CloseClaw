//! Tests for `process_card_media` — audio and file tag handling.
//!
//! Covers:
//! - audio tag with file_token → upload attempted, file_token replaced on success
//! - file tag with file_token → upload attempted, file_token replaced on success
//! - audio tag without file_token → skipped gracefully
//! - file tag without file_token → skipped gracefully
//! - mixed elements (img + media + audio + file) → each processed independently
//! - unknown tag → skipped
//! - img tag: missing img_key skipped; upload failure keeps img_key; success replaces it
//! - file/img with an existing local file + failing CLI → true upload-failure path
//!   (outbound copy happens, upload error swallowed, token/key kept, Ok returned)

use super::*;
use crate::media_store::MediaStore;
use serial_test::serial;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a mock lark-cli that always fails (for testing error handling).
fn create_failing_mock_cli(tmp: &TempDir) -> String {
    use std::io::Write;
    let script_path = tmp.path().join("failing_cli.sh");
    let mut f = std::fs::File::create(&script_path).unwrap();
    writeln!(f, "#!/bin/bash").unwrap();
    writeln!(
        f,
        "echo '{{\"code\":230001,\"msg\":\"capability error\"}}' >&2"
    )
    .unwrap();
    writeln!(f, "echo '{{\"code\":230001,\"msg\":\"capability error\"}}'").unwrap();
    writeln!(f, "exit 1").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
    }
    script_path.to_str().unwrap().to_string()
}

/// Create a mock lark-cli that returns a successful upload response.
///
/// `script_name` keeps the generated scripts distinct (file vs image uploads)
/// and `response_field` names the key returned under `data` (`file_key` for
/// file/audio uploads, `image_key` for image uploads).
fn create_success_upload_mock_cli(
    tmp: &TempDir,
    script_name: &str,
    response_field: &str,
    key: &str,
) -> String {
    use std::io::Write;
    let script_path = tmp.path().join(script_name);
    let mut f = std::fs::File::create(&script_path).unwrap();
    writeln!(f, "#!/bin/bash").unwrap();
    writeln!(
        f,
        "echo '{{\"code\":0,\"msg\":\"ok\",\"data\":{{\"{}\":\"{}\"}}}}'",
        response_field, key
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
    }
    script_path.to_str().unwrap().to_string()
}

/// Create a FeishuPlugin with a mock CLI for testing.
fn make_plugin(cli_command: &str) -> (FeishuPlugin, TempDir) {
    let tmp = TempDir::new().expect("tmp dir");
    let media_store = Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"));
    let mut adapter = FeishuAdapter::new("test_profile".into(), media_store);
    adapter.cli_command = cli_command.to_string();
    let adapter = Arc::new(adapter);
    (FeishuPlugin::new(adapter), tmp)
}

// ===========================================================================
// process_card_media — match arm routing
// ===========================================================================

/// Audio element with file_token: process_card_media extracts and attempts upload.
/// With a failing mock CLI, the upload fails gracefully (warn + continue).
#[tokio::test]
async fn test_process_card_media_audio_element_graceful_failure() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "audio", "file_token": "/tmp/test_audio.mp3" }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(
        result.is_ok(),
        "process_card_media should not fail even when upload fails"
    );

    // file_token should remain unchanged since upload failed
    let elements = payload["card"]["elements"].as_array().unwrap();
    let audio = &elements[0];
    assert_eq!(audio["file_token"], "/tmp/test_audio.mp3");
}

/// File element with file_token: process_card_media extracts and attempts upload.
/// With a failing mock CLI, the upload fails gracefully (warn + continue).
#[tokio::test]
async fn test_process_card_media_file_element_graceful_failure() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "file", "file_token": "/tmp/test_document.pdf" }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(
        result.is_ok(),
        "process_card_media should not fail even when upload fails"
    );

    // file_token should remain unchanged since upload failed
    let elements = payload["card"]["elements"].as_array().unwrap();
    let file = &elements[0];
    assert_eq!(file["file_token"], "/tmp/test_document.pdf");
}

/// Audio element without file_token: skipped gracefully, no panic.
#[tokio::test]
async fn test_process_card_media_audio_no_file_token_skipped() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "audio" }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(result.is_ok());
}

/// File element without file_token: skipped gracefully, no panic.
#[tokio::test]
async fn test_process_card_media_file_no_file_token_skipped() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "file" }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(result.is_ok());
}

/// Mixed elements: audio, file, media, and unknown tag all processed independently.
/// Each processes (or skips) without affecting others.
#[tokio::test]
async fn test_process_card_media_mixed_elements() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "audio", "file_token": "/tmp/audio.mp3" },
                { "tag": "file", "file_token": "/tmp/doc.pdf" },
                { "tag": "media", "file_token": "/tmp/video.mp4" },
                { "tag": "markdown", "content": "text only" },
                { "tag": "unknown_tag", "data": "ignored" }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(result.is_ok());

    // All elements should remain (upload failed for all media types)
    let elements = payload["card"]["elements"].as_array().unwrap();
    assert_eq!(elements.len(), 5);
    assert_eq!(elements[0]["tag"], "audio");
    assert_eq!(elements[0]["file_token"], "/tmp/audio.mp3");
    assert_eq!(elements[1]["tag"], "file");
    assert_eq!(elements[1]["file_token"], "/tmp/doc.pdf");
    assert_eq!(elements[2]["tag"], "media");
    assert_eq!(elements[2]["file_token"], "/tmp/video.mp4");
    assert_eq!(elements[3]["tag"], "markdown");
    assert_eq!(elements[4]["tag"], "unknown_tag");
}

/// Empty card elements: no panic, returns Ok.
#[tokio::test]
async fn test_process_card_media_empty_elements() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({
        "card": {
            "elements": []
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(result.is_ok());
}

/// No card key in payload: returns Ok without error.
#[tokio::test]
async fn test_process_card_media_no_card_key() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({});
    let result = plugin.process_card_media(&mut payload).await;
    assert!(result.is_ok());
}

/// No elements in card: returns Ok without error.
#[tokio::test]
async fn test_process_card_media_no_elements_in_card() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({
        "card": {}
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(result.is_ok());
}

/// Audio element with HTTP URL as file_token: skipped (not a local file).
#[tokio::test]
async fn test_process_card_media_audio_http_url_skipped() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "audio", "file_token": "https://example.com/audio.mp3" }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(result.is_ok());
    // file_token unchanged — HTTP URLs are skipped by try_resolve_media_path
    let elements = payload["card"]["elements"].as_array().unwrap();
    assert_eq!(elements[0]["file_token"], "https://example.com/audio.mp3");
}

/// File element with HTTP URL as file_token: skipped (not a local file).
#[tokio::test]
async fn test_process_card_media_file_http_url_skipped() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "file", "file_token": "https://example.com/doc.pdf" }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(result.is_ok());
    // file_token unchanged — HTTP URLs are skipped by try_resolve_media_path
    let elements = payload["card"]["elements"].as_array().unwrap();
    assert_eq!(elements[0]["file_token"], "https://example.com/doc.pdf");
}

// ===========================================================================
// process_card_media — true upload-failure path (existing local file)
// ===========================================================================

/// File element pointing at an existing whitelisted file with a failing CLI.
///
/// Unlike the skip-path tests above (nonexistent reference → prepare returns
/// None), this one reaches `upload_file`: the outbound copy exists, the upload
/// errors, and the error must be swallowed (warn + continue) — `file_token`
/// stays unchanged and `process_card_media` still returns Ok.
#[tokio::test]
async fn test_process_card_media_file_upload_failure_keeps_token() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let test_file = tmp.path().join("test_document_fail.pdf");
    std::fs::write(&test_file, b"fake pdf content").unwrap();

    let media_store = Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"));
    let outbound_dir = media_store.outbound_dir().to_path_buf();
    let mut adapter = FeishuAdapter::new("test_profile".into(), media_store.clone());
    adapter.cli_command = cli;
    let plugin = FeishuPlugin::new(Arc::new(adapter));

    let file_ref = test_file.to_str().unwrap().to_string();
    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "file", "file_token": file_ref.as_str() }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(
        result.is_ok(),
        "upload failure must not fail process_card_media: {:?}",
        result.err()
    );

    let elements = payload["card"]["elements"].as_array().unwrap();
    assert_eq!(
        elements[0]["file_token"],
        file_ref.as_str(),
        "file_token must stay unchanged when upload fails"
    );
    let outbound_files = std::fs::read_dir(&outbound_dir)
        .map(|d| d.count())
        .unwrap_or(0);
    assert!(
        outbound_files > 0,
        "prepare step must have copied the file to outbound (proves upload path was taken)"
    );
}

// ===========================================================================
// process_card_img — img tag handling (skip / upload failure / success)
// ===========================================================================

/// Img element without img_key: skipped gracefully, no panic, payload untouched.
#[tokio::test]
async fn test_process_card_media_img_no_img_key_skipped() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let (plugin, _tmp) = make_plugin(&cli);

    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "img", "alt": "no key here" }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(
        result.is_ok(),
        "missing img_key must not fail process_card_media: {:?}",
        result.err()
    );
    let elements = payload["card"]["elements"].as_array().unwrap();
    assert!(
        elements[0].get("img_key").is_none(),
        "element without img_key must be left untouched"
    );
}

/// Img element with an existing local file + failing CLI: true upload-failure
/// path — `upload_image` errors, warning is logged, `img_key` unchanged,
/// `process_card_media` still returns Ok.
#[tokio::test]
async fn test_process_card_media_img_upload_failure_keeps_key() {
    let tmp = TempDir::new().unwrap();
    let cli = create_failing_mock_cli(&tmp);
    let test_file = tmp.path().join("test_img_fail.png");
    std::fs::write(&test_file, b"fake png content").unwrap();

    let media_store = Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"));
    let outbound_dir = media_store.outbound_dir().to_path_buf();
    let mut adapter = FeishuAdapter::new("test_profile".into(), media_store.clone());
    adapter.cli_command = cli;
    let plugin = FeishuPlugin::new(Arc::new(adapter));

    let img_ref = test_file.to_str().unwrap().to_string();
    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "img", "img_key": img_ref.as_str() }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(
        result.is_ok(),
        "upload failure must not fail process_card_media: {:?}",
        result.err()
    );

    let elements = payload["card"]["elements"].as_array().unwrap();
    assert_eq!(
        elements[0]["img_key"],
        img_ref.as_str(),
        "img_key must stay unchanged when upload fails"
    );
    let outbound_files = std::fs::read_dir(&outbound_dir)
        .map(|d| d.count())
        .unwrap_or(0);
    assert!(
        outbound_files > 0,
        "prepare step must have copied the image to outbound (proves upload path was taken)"
    );
}

/// Img element with local file + successful image upload: `img_key` is
/// replaced with the platform image_key returned by the upload API.
#[serial]
#[tokio::test]
async fn test_process_card_media_img_upload_success_replaces_key() {
    let tmp = TempDir::new().unwrap();
    let expected_key = "v3_img_ghi789";
    let cli = create_success_upload_mock_cli(&tmp, "success_img_cli.sh", "image_key", expected_key);
    let test_file = tmp.path().join("test_img_ok.png");
    std::fs::write(&test_file, b"fake png content").unwrap();

    let media_store = Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"));
    let mut adapter = FeishuAdapter::new("test_profile".into(), media_store);
    adapter.cli_command = cli;
    let plugin = FeishuPlugin::new(Arc::new(adapter));

    let img_ref = test_file.to_str().unwrap().to_string();
    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "img", "img_key": img_ref.as_str() }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(
        result.is_ok(),
        "img upload should succeed: {:?}",
        result.err()
    );

    let elements = payload["card"]["elements"].as_array().unwrap();
    assert_eq!(
        elements[0]["img_key"], expected_key,
        "img_key should be replaced with the platform image_key after successful upload"
    );
}

// ===========================================================================
// process_card_media — happy path (upload success)
// ===========================================================================

/// Audio element with local file: upload succeeds, file_token replaced with platform key.
#[serial]
#[tokio::test]
async fn test_process_card_media_audio_success() {
    let tmp = TempDir::new().unwrap();
    let expected_key = "v3_file_abc123_audio";
    let cli = create_success_upload_mock_cli(&tmp, "success_cli.sh", "file_key", expected_key);

    // Create a real file within the tmp dir (media store root) so path validation passes.
    let test_file = tmp.path().join("test_audio.mp3");
    std::fs::write(&test_file, b"fake audio content").unwrap();

    // Create plugin with media store rooted at the same tmp dir.
    let media_store = Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"));
    let mut adapter = FeishuAdapter::new("test_profile".into(), media_store);
    adapter.cli_command = cli;
    let plugin = FeishuPlugin::new(Arc::new(adapter));

    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "audio", "file_token": test_file.to_str().unwrap() }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(
        result.is_ok(),
        "process_card_media should succeed on upload"
    );

    let elements = payload["card"]["elements"].as_array().unwrap();
    let audio = &elements[0];
    assert_eq!(
        audio["file_token"], expected_key,
        "file_token should be replaced with the platform key after successful upload"
    );
}

/// File element with local file: upload succeeds, file_token replaced with platform key.
#[serial]
#[tokio::test]
async fn test_process_card_media_file_success() {
    let tmp = TempDir::new().unwrap();
    let expected_key = "v3_file_def456_file";
    let cli = create_success_upload_mock_cli(&tmp, "success_cli.sh", "file_key", expected_key);

    // Create a real file within the tmp dir (media store root) so path validation passes.
    let test_file = tmp.path().join("test_document.pdf");
    std::fs::write(&test_file, b"fake pdf content").unwrap();

    // Create plugin with media store rooted at the same tmp dir.
    let media_store = Arc::new(MediaStore::new(tmp.path().to_str().unwrap()).expect("media store"));
    let mut adapter = FeishuAdapter::new("test_profile".into(), media_store);
    adapter.cli_command = cli;
    let plugin = FeishuPlugin::new(Arc::new(adapter));

    let mut payload = serde_json::json!({
        "card": {
            "elements": [
                { "tag": "file", "file_token": test_file.to_str().unwrap() }
            ]
        }
    });
    let result = plugin.process_card_media(&mut payload).await;
    assert!(
        result.is_ok(),
        "process_card_media should succeed on upload"
    );

    let elements = payload["card"]["elements"].as_array().unwrap();
    let file = &elements[0];
    assert_eq!(
        file["file_token"], expected_key,
        "file_token should be replaced with the platform key after successful upload"
    );
}
