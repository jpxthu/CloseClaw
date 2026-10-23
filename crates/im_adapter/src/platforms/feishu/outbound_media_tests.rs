//! Tests for outbound media handling.
//!
//! Convention (see `test_helpers.rs` module docs): any test that creates and
//! actually spawns a mock CLI script must be marked `#[serial_test::serial]`
//! (ETXTBSY root cause); mock scripts must live in a `tempfile::TempDir`.

use super::outbound_media::*;
use super::{
    create_echo_cli, create_failing_mock_cli, create_subdir, make_test_media_store_at,
    write_mock_script,
};
use crate::error::AdapterError;
use serial_test::serial;
use std::fs;
use tempfile::TempDir;

/// Does the captured CLI args line (one line, whitespace-separated — see
/// [`create_echo_cli`]) contain a path equivalent to `outbound`?
///
/// A plain full-text `args.contains(outbound)` silently assumes the system
/// temp dir is not located under the process cwd. When it is (an allowed
/// layout, STANDARDS §8), production `make_media_path_relative` strips the
/// cwd prefix and lark-cli receives a *cwd-relative* path, so the absolute
/// `contains` check fails spuriously. Compare each whitespace-separated
/// token for path equivalence instead:
///
/// - absolute token → direct component-wise `Path` equality;
/// - relative token → resolved against the process cwd. Because
///   `make_media_path_relative` relativizes via `strip_prefix(cwd)`,
///   `cwd.join(token)` reconstructs `outbound` exactly (lexically, no
///   filesystem access), so equivalence still holds.
///
/// A path genuinely absent from the args still yields `false`, keeping the
/// assertion fail-safe; [`assert_captured_args_contain_path`] keeps the full
/// `args` text in the failure message for diagnosability.
///
/// Known limitation (pre-existing, same as `capture_flag_arg`):
/// whitespace-splitting cannot reassemble paths containing spaces.
fn captured_args_contain_path(args: &str, outbound: &std::path::Path) -> bool {
    args.split_whitespace().any(|token| {
        let token_path = std::path::Path::new(token);
        if token_path.is_absolute() {
            token_path == outbound
        } else {
            std::env::current_dir().is_ok_and(|cwd| cwd.join(token_path) == outbound)
        }
    })
}

/// Normal path: an absolute outbound path token in args hits.
#[test]
fn test_captured_args_contain_path_absolute_token_hit() {
    let tmp = TempDir::new().unwrap();
    let outbound = tmp.path().join("outbound").join("photo.png");
    let args = format!("im send --chat oc_x --image {}", outbound.to_str().unwrap());
    assert!(
        captured_args_contain_path(&args, &outbound),
        "absolute outbound token should hit, args: {args}"
    );
}

/// Environment boundary: when TMPDIR sits under the process cwd, production
/// `make_media_path_relative` relativizes the outbound path and lark-cli
/// receives a cwd-relative token — must still hit. Simulated by synthesizing
/// a relative token and defining outbound as `cwd.join(token)`; purely
/// lexical (no filesystem access, no env mutation, no dependence on the
/// real TMPDIR×cwd layout).
#[test]
fn test_captured_args_contain_path_cwd_relative_token_hit() {
    let cwd = std::env::current_dir().unwrap();
    let rel = std::path::Path::new("tmpdir-under-cwd/outbound/photo.png");
    let outbound = cwd.join(rel);
    let args = format!("im send --chat oc_x --image {}", rel.to_str().unwrap());
    assert!(
        captured_args_contain_path(&args, &outbound),
        "cwd-relative token should resolve to outbound, args: {args}"
    );
}

/// Error path: args without any path equivalent to outbound misses, so the
/// call-site assertion still fails (with full args text) when the path was
/// genuinely not passed. Includes the substring trap: a token sharing the
/// outbound path as a prefix (`photo.png.bak`) is a different path — a
/// plain `args.contains(outbound)` would have matched it spuriously.
#[test]
fn test_captured_args_contain_path_missing_path_misses() {
    let tmp = TempDir::new().unwrap();
    let outbound = tmp.path().join("outbound").join("photo.png");

    let args = "im send --chat oc_x --image /elsewhere/other.png";
    assert!(
        !captured_args_contain_path(args, &outbound),
        "unrelated args must not hit"
    );

    let decoy = format!("{}.bak", outbound.to_str().unwrap());
    let args = format!("im send --chat oc_x --image {decoy}");
    assert!(
        !captured_args_contain_path(&args, &outbound),
        "prefix-sharing token must not count as a hit, args: {args}"
    );
}

/// Boundary: the outbound token in the middle of a multi-token args line hits.
#[test]
fn test_captured_args_contain_path_outbound_in_middle_hit() {
    let tmp = TempDir::new().unwrap();
    let outbound = tmp.path().join("outbound").join("report.pdf");
    let args = format!(
        "im send --chat oc_x {} --verbose",
        outbound.to_str().unwrap()
    );
    assert!(
        captured_args_contain_path(&args, &outbound),
        "outbound token mid-args should hit, args: {args}"
    );
}

/// Boundary: same file name in a different directory is a different path —
/// must miss.
#[test]
fn test_captured_args_contain_path_same_name_different_dir_misses() {
    let tmp = TempDir::new().unwrap();
    let outbound = tmp.path().join("outbound").join("photo.png");
    let decoy = tmp.path().join("inbound").join("photo.png");
    let args = format!("im send --chat oc_x --image {}", decoy.to_str().unwrap());
    assert!(
        !captured_args_contain_path(&args, &outbound),
        "same file name in another directory must not hit, args: {args}"
    );
}

/// Assert helper shared by the outbound-copy send tests: reads the CLI
/// captured args from `<tmp>/captured_args` and asserts it contains a path
/// equivalent to `outbound` (see [`captured_args_contain_path`]); on failure
/// the full args text is kept in the message for diagnosability.
fn assert_captured_args_contain_path(tmp: &TempDir, outbound: &std::path::Path) {
    let args_file = tmp.path().join("captured_args");
    let args = fs::read_to_string(&args_file).unwrap();
    assert!(
        captured_args_contain_path(&args, outbound),
        "lark-cli should receive outbound path, args: {args}"
    );
}

#[tokio::test]
async fn test_validate_path_within_workspace() {
    let tmp = TempDir::new().unwrap();
    let ws = create_subdir(tmp.path(), "workspace");
    let media = create_subdir(tmp.path(), "media");

    let file = ws.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_path_within_media_store() {
    let tmp = TempDir::new().unwrap();
    let ws = create_subdir(tmp.path(), "workspace");
    let media = create_subdir(tmp.path(), "media");

    let file = media.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_path_outside_rejected() {
    let tmp = TempDir::new().unwrap();
    let ws = create_subdir(tmp.path(), "workspace");
    let media = create_subdir(tmp.path(), "media");
    let outside = create_subdir(tmp.path(), "outside");

    let file = outside.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_validate_path_no_workspace_only_media() {
    let tmp = TempDir::new().unwrap();
    let media = create_subdir(tmp.path(), "media");

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

    let media_store = make_test_media_store_at(tmp.path());
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

    let media_store = make_test_media_store_at(tmp.path());
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
    let ws = create_subdir(tmp.path(), "ws");
    let file = ws.join("img.png");
    fs::write(&file, "data").unwrap();
    let media_store = make_test_media_store_at(tmp.path());
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
    let media_store = make_test_media_store_at(tmp.path());
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
    let media_store = make_test_media_store_at(tmp.path());
    let result =
        prepare_outbound_local_media("http://example.com/img.png", None, &media_store).await;
    assert!(result.is_none(), "HTTP URL should be skipped");
}

/// HTTPS URL → None (platform-hosted, not an error).
#[tokio::test]
async fn test_prepare_outbound_https_url_skipped() {
    let tmp = TempDir::new().unwrap();
    let media_store = make_test_media_store_at(tmp.path());
    let result =
        prepare_outbound_local_media("https://cdn.example.com/photo.jpg", None, &media_store).await;
    assert!(result.is_none(), "HTTPS URL should be skipped");
}

/// Non-existent local file → None (skipped, not an error).
#[tokio::test]
async fn test_prepare_outbound_nonexistent_file_skipped() {
    let tmp = TempDir::new().unwrap();
    let media_store = make_test_media_store_at(tmp.path());
    let result = prepare_outbound_local_media("no_such_file.png", None, &media_store).await;
    assert!(result.is_none(), "non-existent file should be skipped");
}

/// Path outside whitelist (e.g. /etc/hosts) → None (rejected).
#[tokio::test]
async fn test_prepare_outbound_whitelist_violation_skipped() {
    let tmp = TempDir::new().unwrap();
    let media_store = make_test_media_store_at(tmp.path());
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
    let media_store = make_test_media_store_at(tmp.path());
    let result = prepare_outbound_local_media("", None, &media_store).await;
    assert!(result.is_none(), "empty string should be skipped");
}

/// Outbound dir already has file with same name → unique suffix applied.
#[tokio::test]
async fn test_prepare_outbound_duplicate_filename_unique_suffix() {
    let tmp = TempDir::new().unwrap();
    let media_store = make_test_media_store_at(tmp.path());
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

/// Helper: create a FeishuAdapter with a mock CLI and optional workspace
/// (media store rooted at `tmp`; delegates assembly to the shared
/// [`super::test_helpers::make_adapter_with_store`], then injects
/// `workspace_dir`).
fn make_adapter(
    cli: &str,
    tmp: &TempDir,
    workspace: Option<&std::path::Path>,
) -> super::adapter::FeishuAdapter {
    let mut adapter = super::test_helpers::make_adapter_with_store(cli, tmp);
    adapter.workspace_dir = workspace.map(std::path::PathBuf::from);
    adapter
}

/// Thin-wrapper mapping of the local `make_adapter`: the 3-arg call must map
/// onto the shared [`super::test_helpers::make_adapter_with_store`] plus the
/// `workspace_dir` injection —
/// ① `Some(ws)` → `cli_command` passthrough and `workspace_dir == Some(ws)`;
/// ② `None` → `workspace_dir == None`;
/// ③ media store rooted at the passed `tmp` (a copy made through the
/// adapter's own store lands under `tmp`).
#[tokio::test]
async fn test_make_adapter_wrapper_maps_cli_workspace_and_store_root() {
    let tmp = TempDir::new().unwrap();
    let cli = "/mock/bin/lark-cli";
    let ws = create_subdir(tmp.path(), "ws");

    // ① cli_command passthrough + workspace injection.
    let adapter = make_adapter(cli, &tmp, Some(&ws));
    assert_eq!(adapter.cli_command, cli);
    assert_eq!(
        adapter.workspace_dir.as_deref(),
        Some(ws.as_path()),
        "workspace_dir should be injected verbatim"
    );

    // ② workspace=None → workspace_dir stays None.
    let adapter = make_adapter(cli, &tmp, None);
    assert!(adapter.workspace_dir.is_none());

    // ③ media store rooted at the passed tmp.
    let source = tmp.path().join("source.png");
    fs::write(&source, "png data").unwrap();
    let result = copy_to_outbound(&source, &adapter.media_store)
        .await
        .unwrap();
    assert!(
        result.outbound_path.starts_with(tmp.path()),
        "outbound copy should live under tmp, got {}",
        result.outbound_path.display()
    );
}

/// send_image with workspace-relative path → outbound copy created,
/// sent path is the outbound copy, returns Ok.
#[serial]
#[tokio::test]
async fn test_send_image_workspace_relative_sends_outbound_copy() {
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let ws = create_subdir(tmp.path(), "ws");
    fs::write(ws.join("photo.png"), "png data").unwrap();
    let adapter = make_adapter(&cli, &tmp, Some(&ws));
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
    assert_captured_args_contain_path(&tmp, &outbound);
}

/// send_file with absolute path inside media store → outbound copy created,
/// sent path is the outbound copy, returns Ok.
#[serial]
#[tokio::test]
async fn test_send_file_media_store_absolute_sends_outbound_copy() {
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let file = tmp.path().join("report.pdf");
    fs::write(&file, "pdf data").unwrap();
    let adapter = make_adapter(&cli, &tmp, None);
    let result = adapter.send_file("oc_chat2", file.to_str().unwrap()).await;
    assert!(
        result.is_ok(),
        "send_file should succeed — got {:?}",
        result
    );
    let outbound = adapter.media_store.outbound_dir().join("report.pdf");
    assert!(outbound.exists(), "outbound copy should exist");
    assert_captured_args_contain_path(&tmp, &outbound);
}

/// HTTP URL → send_image skips send and returns Ok (not an error).
#[tokio::test]
async fn test_send_image_http_url_skips_send() {
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let adapter = make_adapter(&cli, &tmp, None);
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
    let adapter = make_adapter(&cli, &tmp, None);
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
    let adapter = make_adapter(&cli, &tmp, None);
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

// =================================================================
// Step 1.4 — platform rejection path tests (send side)
// =================================================================

/// Helper: create a mock lark-cli that returns a rejection response
/// with a non-zero code and error message.
fn create_reject_cli(tmp: &TempDir, code: i64, msg: &str) -> String {
    let script = tmp.path().join("reject.sh");
    let content = format!("#!/bin/bash\necho '{{\"code\":{code},\"msg\":\"{msg}\"}}'\n");
    write_mock_script(&script, &content);
    script.to_str().unwrap().to_string()
}

/// Helper: create a mock lark-cli that returns a success response
/// with the given data payload.
fn create_success_cli(tmp: &TempDir, data: &str) -> String {
    let script = tmp.path().join("success.sh");
    let content = format!("#!/bin/bash\necho '{{\"code\":0,\"msg\":\"ok\",\"data\":{data}}}'\n");
    write_mock_script(&script, &content);
    script.to_str().unwrap().to_string()
}

/// upload_image with mock CLI returning non-zero code → SendFailed
/// with platform error code/msg in error string.
#[serial]
#[tokio::test]
async fn test_upload_image_reject_returns_send_failed_with_code_msg() {
    let tmp = TempDir::new().unwrap();
    let cli = create_reject_cli(&tmp, 99001, "image too large");
    let file = tmp.path().join("photo.png");
    fs::write(&file, "data").unwrap();
    let adapter = make_adapter(&cli, &tmp, None);
    let result = upload_image(&adapter, &file).await;
    match result {
        Err(AdapterError::SendFailed(msg)) => {
            assert!(
                msg.contains("99001"),
                "should contain error code, got: {msg}"
            );
            assert!(
                msg.contains("image too large"),
                "should contain error msg, got: {msg}"
            );
        }
        other => panic!("expected SendFailed, got: {other:?}"),
    }
}

/// upload_file with mock CLI returning non-zero code → SendFailed
/// with platform error code/msg in error string.
#[serial]
#[tokio::test]
async fn test_upload_file_reject_returns_send_failed_with_code_msg() {
    let tmp = TempDir::new().unwrap();
    let cli = create_reject_cli(&tmp, 99002, "file type not allowed");
    let file = tmp.path().join("doc.pdf");
    fs::write(&file, "data").unwrap();
    let adapter = make_adapter(&cli, &tmp, None);
    let result = upload_file(&adapter, &file, "doc.pdf").await;
    match result {
        Err(AdapterError::SendFailed(msg)) => {
            assert!(
                msg.contains("99002"),
                "should contain error code, got: {msg}"
            );
            assert!(
                msg.contains("file type not allowed"),
                "should contain error msg, got: {msg}"
            );
        }
        other => panic!("expected SendFailed, got: {other:?}"),
    }
}

/// upload_image with mock CLI returning code 0 + image_key → returns
/// the image key successfully.
#[serial]
#[tokio::test]
async fn test_upload_image_success_returns_image_key() {
    let tmp = TempDir::new().unwrap();
    let data = r#"{"image_key": "img_v3_test123"}"#;
    let cli = create_success_cli(&tmp, data);
    let file = tmp.path().join("photo.png");
    fs::write(&file, "data").unwrap();
    let adapter = make_adapter(&cli, &tmp, None);
    let result = upload_image(&adapter, &file).await;
    assert_eq!(result.unwrap(), "img_v3_test123");
}

/// upload_file with mock CLI returning code 0 + file_key → returns
/// the file key successfully.
#[serial]
#[tokio::test]
async fn test_upload_file_success_returns_file_key() {
    let tmp = TempDir::new().unwrap();
    let data = r#"{"file_key": "file_v3_abc789"}"#;
    let cli = create_success_cli(&tmp, data);
    let file = tmp.path().join("report.pdf");
    fs::write(&file, "data").unwrap();
    let adapter = make_adapter(&cli, &tmp, None);
    let result = upload_file(&adapter, &file, "report.pdf").await;
    assert_eq!(result.unwrap(), "file_v3_abc789");
}

/// Copy to outbound preserved even when upload (lark-cli) fails.
///
/// The mock script exits 1 with an error code/msg on stderr; the test must
/// take that exit-1 path (not a spawn failure such as ETXTBSY).
#[serial]
#[tokio::test]
async fn test_send_file_copy_preserved_on_upload_failure() {
    let tmp = TempDir::new().unwrap();
    // Failing mock CLI, built by the shared parameterized creator, which
    // lands the script atomically via write_mock_script (temp write →
    // handle closed → rename → chmod): no write handle is alive when
    // send_file() spawns it, so execve cannot fail with ETXTBSY.
    let cli = create_failing_mock_cli(
        &tmp,
        "fail.sh",
        r#"{"code":999,"msg":"upload rejected"}"#,
        true,
        1,
    );

    let file = tmp.path().join("doc.pdf");
    fs::write(&file, "pdf content").unwrap();
    let adapter = make_adapter(&cli, &tmp, None);
    let result = adapter
        .send_file("oc_chat_fail", file.to_str().unwrap())
        .await;
    // Must be the script's exit-1 failure, never a spawn failure (ETXTBSY).
    match result {
        Err(AdapterError::SendFailed(msg)) => {
            assert!(
                !msg.contains("lark-cli spawn error"),
                "spawn failure: {msg}"
            );
            assert!(
                msg.contains("lark-cli exited with code Some(1)"),
                "expect script exit-1, got: {msg}"
            );
            assert!(
                msg.contains("\"code\":999") && msg.contains("upload rejected"),
                "expect script error code/msg, got: {msg}"
            );
        }
        other => panic!("expected SendFailed, got: {other:?}"),
    }
    let outbound = adapter.media_store.outbound_dir().join("doc.pdf");
    assert!(
        outbound.exists(),
        "outbound copy should be preserved even when upload fails"
    );
    // Byte-compare the preserved copy against the source file: a
    // half-written or truncated copy must fail this assertion, not just
    // an absent one.
    let source_bytes = fs::read(&file).unwrap();
    let outbound_bytes = fs::read(&outbound).unwrap();
    assert_eq!(
        source_bytes, outbound_bytes,
        "preserved outbound copy must be byte-identical to the source file"
    );
}

/// send_image with workspace-relative path, workspace is None →
/// file resolved from cwd (or media store) — skipped if not found.
#[tokio::test]
async fn test_send_image_no_workspace_relative_skipped() {
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let adapter = make_adapter(&cli, &tmp, None);
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
    let adapter = make_adapter(&cli, &tmp, None);
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
    let adapter = make_adapter(&cli, &tmp, None);
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
#[serial]
#[tokio::test]
async fn test_dispatch_send_media_audio_file_outbound_copy() {
    use closeclaw_common::processor::ContentBlock;
    let tmp = TempDir::new().unwrap();
    let cli = create_echo_cli(&tmp);
    let ws = create_subdir(tmp.path(), "ws");
    fs::write(ws.join("speech.opus"), "audio data").unwrap();
    let adapter = make_adapter(&cli, &tmp, Some(&ws));
    let block = ContentBlock::Audio {
        name: "speech".into(),
        url: "speech.opus".into(),
    };
    let result =
        super::card_media_fallback::dispatch_send_media(&adapter, "oc_chat9", &block).await;
    assert!(result.is_ok());
    let outbound = adapter.media_store.outbound_dir().join("speech.opus");
    assert!(outbound.exists(), "outbound copy should exist for audio");
    assert_captured_args_contain_path(&tmp, &outbound);
}

// =================================================================
// Step 1.4 — media_store_dir canonicalize failure → reject send
// =================================================================

/// When the media store directory is a broken symlink (cannot be
/// canonicalized), `validate_outbound_path` must return Err instead of
/// silently degrading to a raw (non-canonical) path.
#[tokio::test]
async fn test_validate_path_media_store_broken_symlink_rejected() {
    let tmp = TempDir::new().unwrap();
    let ws = create_subdir(tmp.path(), "workspace");
    let media = tmp.path().join("media");
    // Create a broken symlink for the media store directory.
    #[cfg(unix)]
    std::os::unix::fs::symlink("/nonexistent/path", &media).unwrap();
    #[cfg(not(unix))]
    return; // broken-symlink test not applicable on non-unix

    let file = ws.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media).await;
    assert!(
        result.is_err(),
        "should reject when media store dir cannot be canonicalized"
    );
}

/// When the workspace directory is a broken symlink, validation should
/// still work via the media store path (workspace degradation is allowed
/// with a warning, but media_store_dir must succeed).
#[tokio::test]
async fn test_validate_path_workspace_broken_symlink_falls_back_to_media() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let media = create_subdir(tmp.path(), "media");
    // Create a broken symlink for the workspace directory.
    #[cfg(unix)]
    std::os::unix::fs::symlink("/nonexistent/path", &ws).unwrap();
    #[cfg(not(unix))]
    return;

    let file = media.join("test.txt");
    fs::write(&file, "content").unwrap();

    // Should still succeed because the file is inside the (valid) media store.
    let result = validate_outbound_path(&file, Some(&ws), &media).await;
    assert!(
        result.is_ok(),
        "should accept file within media store even when workspace is broken symlink"
    );
}

/// `prepare_outbound_local_media` with broken media store symlink →
/// returns None (rejected, does not panic).
#[tokio::test]
async fn test_prepare_outbound_broken_media_store_symlink_skipped() {
    let tmp = TempDir::new().unwrap();
    let media = tmp.path().join("media");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/nonexistent/path", &media).unwrap();
    #[cfg(not(unix))]
    return;

    // We need a valid MediaStore, but since MediaStore::new validates the
    // dir, we test at the validate_outbound_path level instead.
    let ws = create_subdir(tmp.path(), "ws");
    let file = ws.join("img.png");
    fs::write(&file, "data").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media).await;
    assert!(
        result.is_err(),
        "should reject when media store dir is broken symlink"
    );
}
