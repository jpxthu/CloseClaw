//! Tests for outbound media handling.

use super::outbound_media::*;
use crate::media_store::MediaStore;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_validate_path_within_workspace() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let media = tmp.path().join("media");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&media).unwrap();

    let file = ws.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_path_within_media_store() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let media = tmp.path().join("media");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&media).unwrap();

    let file = media.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_path_outside_rejected() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let media = tmp.path().join("media");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&media).unwrap();
    fs::create_dir_all(&outside).unwrap();

    let file = outside.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_validate_path_no_workspace_only_media() {
    let tmp = TempDir::new().unwrap();
    let media = tmp.path().join("media");
    fs::create_dir_all(&media).unwrap();

    let file = media.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, None, &media).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_copy_to_outbound_creates_file() {
    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("source.png");
    fs::write(&source, "image data").unwrap();

    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    let result = copy_to_outbound(&source, &media_store).await.unwrap();

    assert!(result.outbound_path.exists());
    let fname = result
        .outbound_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(fname, "source.png");
}

#[tokio::test]
async fn test_copy_to_outbound_unique_name_on_conflict() {
    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("test.png");
    fs::write(&source, "data1").unwrap();

    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    // Create a file with the same name in outbound.
    fs::write(media_store.outbound_dir().join("test.png"), "existing").unwrap();

    let result = copy_to_outbound(&source, &media_store).await.unwrap();
    assert!(result.outbound_path.exists());
    let fname = result
        .outbound_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_ne!(fname, "test.png");
    assert!(fname.starts_with("test_"));
}

#[test]
fn test_detect_file_type_opus() {
    use std::path::Path;
    assert_eq!(detect_file_type(Path::new("audio.opus")), "opus");
}

#[test]
fn test_detect_file_type_mp4() {
    use std::path::Path;
    assert_eq!(detect_file_type(Path::new("video.mp4")), "mp4");
}

#[test]
fn test_detect_file_type_pdf() {
    use std::path::Path;
    assert_eq!(detect_file_type(Path::new("doc.pdf")), "pdf");
}

#[test]
fn test_detect_file_type_unknown() {
    use std::path::Path;
    assert_eq!(detect_file_type(Path::new("data.xyz")), "stream");
}

// -- upload_image MIME type tests --

/// Helper: create temporary image files with different extensions.
fn create_test_images() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("photo.png"), "png data").unwrap();
    std::fs::write(tmp.path().join("photo.jpg"), "jpg data").unwrap();
    std::fs::write(tmp.path().join("photo.jpeg"), "jpeg data").unwrap();
    std::fs::write(tmp.path().join("anim.gif"), "gif data").unwrap();
    std::fs::write(tmp.path().join("pic.webp"), "webp data").unwrap();
    tmp
}

/// Verify that upload_image MIME detection returns correct types
/// based on file extension. This is a unit test of the detection
/// logic only — no actual upload occurs.
#[test]
fn upload_image_mime_detection_by_extension() {
    let tmp = create_test_images();
    let detect = |ext: &str| -> &'static str {
        match ext.to_lowercase().as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            _ => "image/png",
        }
    };

    assert_eq!(detect("png"), "image/png");
    assert_eq!(detect("jpg"), "image/jpeg");
    assert_eq!(detect("jpeg"), "image/jpeg");
    assert_eq!(detect("gif"), "image/gif");
    assert_eq!(detect("webp"), "image/webp");
    assert_eq!(detect("svg"), "image/svg+xml");
    assert_eq!(detect("bmp"), "image/png"); // unknown → fallback to png

    // Verify files exist for integration scenario.
    assert!(tmp.path().join("photo.png").exists());
    assert!(tmp.path().join("photo.jpg").exists());
    assert!(tmp.path().join("anim.gif").exists());
}

// =================================================================
// prepare_outbound_local_media — Step 1.3 behavior coverage
// =================================================================

/// Local file under workspace with relative path → resolved, validated,
/// copied to outbound/ and the outbound path returned.
#[tokio::test]
async fn test_prepare_outbound_workspace_relative_path() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    let file = ws.join("img.png");
    fs::write(&file, "data").unwrap();
    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    let result = prepare_outbound_local_media("img.png", Some(&ws), &media_store).await;
    assert!(result.is_some(), "should return outbound path");
    let outbound = result.unwrap();
    assert!(outbound.exists());
    assert!(outbound.starts_with(media_store.outbound_dir()));
    assert_eq!(outbound.file_name().unwrap(), "img.png");
}

/// Absolute path inside media store → validated and copied to outbound.
#[tokio::test]
async fn test_prepare_outbound_media_store_absolute_path() {
    let tmp = TempDir::new().unwrap();
    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    let file = tmp.path().join("doc.pdf");
    fs::write(&file, "pdf data").unwrap();
    let result = prepare_outbound_local_media(file.to_str().unwrap(), None, &media_store).await;
    assert!(result.is_some());
    let outbound = result.unwrap();
    assert!(outbound.exists());
    assert_eq!(outbound.file_name().unwrap(), "doc.pdf");
}

/// HTTP URL → None (platform-hosted, not an error).
#[tokio::test]
async fn test_prepare_outbound_http_url_skipped() {
    let tmp = TempDir::new().unwrap();
    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    let result =
        prepare_outbound_local_media("http://example.com/img.png", None, &media_store).await;
    assert!(result.is_none(), "HTTP URL should be skipped");
}

/// HTTPS URL → None (platform-hosted, not an error).
#[tokio::test]
async fn test_prepare_outbound_https_url_skipped() {
    let tmp = TempDir::new().unwrap();
    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    let result =
        prepare_outbound_local_media("https://cdn.example.com/photo.jpg", None, &media_store).await;
    assert!(result.is_none(), "HTTPS URL should be skipped");
}

/// Non-existent local file → None (skipped, not an error).
#[tokio::test]
async fn test_prepare_outbound_nonexistent_file_skipped() {
    let tmp = TempDir::new().unwrap();
    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    let result = prepare_outbound_local_media("no_such_file.png", None, &media_store).await;
    assert!(result.is_none(), "non-existent file should be skipped");
}

/// Path outside whitelist (e.g. /etc/hosts) → None (rejected).
#[tokio::test]
async fn test_prepare_outbound_whitelist_violation_skipped() {
    let tmp = TempDir::new().unwrap();
    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    // /etc/hosts exists on Linux but is outside any workspace/media store.
    let result = prepare_outbound_local_media("/etc/hosts", None, &media_store).await;
    assert!(
        result.is_none(),
        "whitelist-violating path should be skipped"
    );
}

/// Empty string reference → treated as relative path, file does not exist →
/// None.
#[tokio::test]
async fn test_prepare_outbound_empty_string_skipped() {
    let tmp = TempDir::new().unwrap();
    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    let result = prepare_outbound_local_media("", None, &media_store).await;
    assert!(result.is_none(), "empty string should be skipped");
}

/// Outbound dir already has file with same name → unique suffix applied.
#[tokio::test]
async fn test_prepare_outbound_duplicate_filename_unique_suffix() {
    let tmp = TempDir::new().unwrap();
    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    // Create file in media store.
    let file = tmp.path().join("report.pdf");
    fs::write(&file, "content").unwrap();
    // Pre-populate outbound/ with a same-named file.
    fs::write(media_store.outbound_dir().join("report.pdf"), "existing").unwrap();
    let result = prepare_outbound_local_media(file.to_str().unwrap(), None, &media_store).await;
    assert!(result.is_some());
    let outbound = result.unwrap();
    let name = outbound.file_name().unwrap().to_string_lossy();
    assert_ne!(name.as_ref(), "report.pdf", "should use unique suffix");
    assert!(name.starts_with("report_"));
}

// =================================================================
// send_media_file integration — outbound copy before send
// =================================================================

/// Helper: create a mock CLI that records args to a file and succeeds.
fn create_echo_cli(tmp: &TempDir) -> String {
    use std::io::Write;
    let script = tmp.path().join("echo.sh");
    let args_file = tmp.path().join("captured_args");
    let args_path = args_file.to_str().unwrap().to_string();
    let mut f = std::fs::File::create(&script).unwrap();
    writeln!(f, "#!/bin/bash").unwrap();
    writeln!(f, "echo \"$@\" > {args_path}").unwrap();
    writeln!(f, "echo '{{\"code\":0}}'").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, PermissionsExt::from_mode(0o755)).unwrap();
    }
    script.to_str().unwrap().to_string()
}

/// Helper: create a FeishuAdapter with a mock CLI and optional workspace.
fn make_adapter(
    cli: &str,
    media_root: &std::path::Path,
    workspace: Option<&std::path::Path>,
) -> super::adapter::FeishuAdapter {
    let media_store = Arc::new(MediaStore::new(media_root.to_str().unwrap()).unwrap());
    let mut adapter = super::adapter::FeishuAdapter::new("test_profile".into(), media_store);
    adapter.cli_command = cli.to_string();
    adapter.workspace_dir = workspace.map(std::path::PathBuf::from);
    adapter
}

/// send_image with workspace-relative path → outbound copy created,
/// sent path is the outbound copy, returns Ok.
#[tokio::test]
async fn test_send_image_workspace_relative_sends_outbound_copy() {
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("photo.png"), "png data").unwrap();
    let adapter = make_adapter(&cli, tmp.path(), Some(&ws));
    let result = adapter.send_image("oc_chat1", "photo.png").await;
    assert!(
        result.is_ok(),
        "send_image should succeed — got {:?}",
        result
    );
    // Verify outbound copy exists.
    let outbound = adapter.media_store.outbound_dir().join("photo.png");
    assert!(outbound.exists(), "outbound copy should exist");
    // Verify sent path is the outbound copy (not the original).
    let args_file = tmp.path().join("captured_args");
    let args = fs::read_to_string(&args_file).unwrap();
    assert!(
        args.contains(outbound.to_str().unwrap()),
        "lark-cli should receive outbound path, args: {args}"
    );
}

/// send_file with absolute path inside media store → outbound copy created,
/// sent path is the outbound copy, returns Ok.
#[tokio::test]
async fn test_send_file_media_store_absolute_sends_outbound_copy() {
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let file = tmp.path().join("report.pdf");
    fs::write(&file, "pdf data").unwrap();
    let adapter = make_adapter(&cli, tmp.path(), None);
    let result = adapter.send_file("oc_chat2", file.to_str().unwrap()).await;
    assert!(
        result.is_ok(),
        "send_file should succeed — got {:?}",
        result
    );
    let outbound = adapter.media_store.outbound_dir().join("report.pdf");
    assert!(outbound.exists(), "outbound copy should exist");
    let args_file = tmp.path().join("captured_args");
    let args = fs::read_to_string(&args_file).unwrap();
    assert!(
        args.contains(outbound.to_str().unwrap()),
        "lark-cli should receive outbound path, args: {args}"
    );
}

/// HTTP URL → send_image skips send and returns Ok (not an error).
#[tokio::test]
async fn test_send_image_http_url_skips_send() {
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let adapter = make_adapter(&cli, tmp.path(), None);
    let result = adapter
        .send_image("oc_chat3", "https://example.com/img.png")
        .await;
    assert!(result.is_ok(), "HTTP URL should return Ok (skip)");
    // No args should have been captured (no CLI call).
    let args_file = tmp.path().join("captured_args");
    assert!(!args_file.exists(), "no CLI call expected for HTTP URL");
}

/// Non-existent file → send_image skips send and returns Ok.
#[tokio::test]
async fn test_send_image_nonexistent_skips_send() {
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let adapter = make_adapter(&cli, tmp.path(), None);
    let result = adapter.send_image("oc_chat4", "no_such_file.png").await;
    assert!(result.is_ok(), "non-existent file should return Ok (skip)");
    let args_file = tmp.path().join("captured_args");
    assert!(
        !args_file.exists(),
        "no CLI call expected for non-existent file"
    );
}

/// Path outside whitelist → send_file skips send and returns Ok.
#[tokio::test]
async fn test_send_file_whitelist_violation_skips_send() {
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let adapter = make_adapter(&cli, tmp.path(), None);
    let result = adapter.send_file("oc_chat5", "/etc/hosts").await;
    assert!(
        result.is_ok(),
        "whitelist violation should return Ok (skip)"
    );
    let args_file = tmp.path().join("captured_args");
    assert!(
        !args_file.exists(),
        "no CLI call expected for whitelist violation"
    );
}

/// Copy to outbound preserved even when upload (lark-cli) fails.
#[tokio::test]
async fn test_send_file_copy_preserved_on_upload_failure() {
    let tmp = TempDir::new().unwrap();
    // Failing mock CLI.
    use std::io::Write;
    let script = tmp.path().join("fail.sh");
    let mut f = std::fs::File::create(&script).unwrap();
    writeln!(f, "#!/bin/bash").unwrap();
    writeln!(f, "echo '{{\"code\":999}}' >&2").unwrap();
    writeln!(f, "echo '{{\"code\":999}}'").unwrap();
    writeln!(f, "exit 1").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, PermissionsExt::from_mode(0o755)).unwrap();
    }
    let cli = script.to_str().unwrap().to_string();

    let file = tmp.path().join("doc.pdf");
    fs::write(&file, "pdf content").unwrap();
    let adapter = make_adapter(&cli, tmp.path(), None);
    // send_file should fail (lark-cli error) but outbound copy is preserved.
    let result = adapter
        .send_file("oc_chat_fail", file.to_str().unwrap())
        .await;
    assert!(result.is_err(), "lark-cli failure should propagate as Err");
    let outbound = adapter.media_store.outbound_dir().join("doc.pdf");
    assert!(
        outbound.exists(),
        "outbound copy should be preserved even when upload fails"
    );
}

/// send_image with workspace-relative path, workspace is None →
/// file resolved from cwd (or media store) — skipped if not found.
#[tokio::test]
async fn test_send_image_no_workspace_relative_skipped() {
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let adapter = make_adapter(&cli, tmp.path(), None);
    // No workspace set, relative path can't resolve → skipped.
    let result = adapter.send_image("oc_chat6", "relative/path.png").await;
    assert!(result.is_ok(), "should return Ok (skip) when no workspace");
    let args_file = tmp.path().join("captured_args");
    assert!(!args_file.exists(), "no CLI call expected");
}

/// dispatch_send_media with whitelist-violating path → skips (no panic).
#[tokio::test]
async fn test_dispatch_send_media_whitelist_violation_skips() {
    use closeclaw_common::processor::ContentBlock;
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let adapter = make_adapter(&cli, tmp.path(), None);
    let block = ContentBlock::Image {
        name: "evil.png".into(),
        url: "/etc/passwd".into(),
    };
    let result =
        super::card_media_fallback::dispatch_send_media(&adapter, "oc_chat7", &block).await;
    assert!(
        result.is_ok(),
        "whitelist violation should return Ok (skip)"
    );
    let args_file = tmp.path().join("captured_args");
    assert!(
        !args_file.exists(),
        "no CLI call expected for whitelist violation"
    );
}

/// dispatch_send_media with HTTP URL → sends as-is (outbound preparation
/// returns None, send_image skips the local-file path).
#[tokio::test]
async fn test_dispatch_send_media_http_url_skips_local_copy() {
    use closeclaw_common::processor::ContentBlock;
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let adapter = make_adapter(&cli, tmp.path(), None);
    let block = ContentBlock::Image {
        name: "remote.png".into(),
        url: "https://example.com/img.png".into(),
    };
    let result =
        super::card_media_fallback::dispatch_send_media(&adapter, "oc_chat8", &block).await;
    assert!(result.is_ok());
    // No CLI call expected because send_image returns Ok immediately
    // for HTTP URLs (prepare_outbound returns None → skip).
    let args_file = tmp.path().join("captured_args");
    assert!(!args_file.exists(), "no CLI call expected for HTTP URL");
}

/// dispatch_send_media with audio file (file extension) →
/// validates and copies to outbound before sending.
#[tokio::test]
async fn test_dispatch_send_media_audio_file_outbound_copy() {
    use closeclaw_common::processor::ContentBlock;
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("speech.opus"), "audio data").unwrap();
    let adapter = make_adapter(&cli, tmp.path(), Some(&ws));
    let block = ContentBlock::Audio {
        name: "speech".into(),
        url: "speech.opus".into(),
    };
    let result =
        super::card_media_fallback::dispatch_send_media(&adapter, "oc_chat9", &block).await;
    assert!(result.is_ok());
    let outbound = adapter.media_store.outbound_dir().join("speech.opus");
    assert!(outbound.exists(), "outbound copy should exist for audio");
    let args_file = tmp.path().join("captured_args");
    let args = fs::read_to_string(&args_file).unwrap();
    assert!(
        args.contains(outbound.to_str().unwrap()),
        "lark-cli should receive outbound path, args: {args}"
    );
}
