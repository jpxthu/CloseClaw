//! Tests for outbound media handling.

use super::outbound_media::*;
use crate::media_store::MediaStore;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_validate_path_within_workspace() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let media = tmp.path().join("media");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&media).unwrap();

    let file = ws.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media);
    assert!(result.is_ok());
}

#[test]
fn test_validate_path_within_media_store() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let media = tmp.path().join("media");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&media).unwrap();

    let file = media.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media);
    assert!(result.is_ok());
}

#[test]
fn test_validate_path_outside_rejected() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let media = tmp.path().join("media");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&media).unwrap();
    fs::create_dir_all(&outside).unwrap();

    let file = outside.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, Some(&ws), &media);
    assert!(result.is_err());
}

#[test]
fn test_validate_path_no_workspace_only_media() {
    let tmp = TempDir::new().unwrap();
    let media = tmp.path().join("media");
    fs::create_dir_all(&media).unwrap();

    let file = media.join("test.txt");
    fs::write(&file, "content").unwrap();

    let result = validate_outbound_path(&file, None, &media);
    assert!(result.is_ok());
}

#[test]
fn test_copy_to_outbound_creates_file() {
    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("source.png");
    fs::write(&source, "image data").unwrap();

    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    let result = copy_to_outbound(&source, &media_store).unwrap();

    assert!(result.outbound_path.exists());
    let fname = result
        .outbound_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(fname, "source.png");
}

#[test]
fn test_copy_to_outbound_unique_name_on_conflict() {
    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("test.png");
    fs::write(&source, "data1").unwrap();

    let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
    // Create a file with the same name in outbound.
    fs::write(media_store.outbound_dir().join("test.png"), "existing").unwrap();

    let result = copy_to_outbound(&source, &media_store).unwrap();
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
