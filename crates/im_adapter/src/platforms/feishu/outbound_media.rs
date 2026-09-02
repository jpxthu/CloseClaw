//! Outbound media handling for the Feishu adapter.
//!
//! Handles:
//! - Source path whitelist validation (workspace + media storage only)
//! - Copying media to outbound/ subdirectory
//! - Uploading media files to Feishu

use std::path::{Path, PathBuf};

use crate::error::AdapterError;
use crate::media_store::MediaStore;

use super::adapter::FeishuAdapter;
use tracing::warn;

/// Maximum file size for Feishu image upload (10 MB).
const MAX_IMAGE_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum file size for Feishu file upload (30 MB).
const MAX_FILE_SIZE: u64 = 30 * 1024 * 1024;

/// Outbound media validation and copy result.
pub(crate) struct OutboundMediaResult {
    /// Absolute path to the copied file in the outbound directory.
    pub outbound_path: PathBuf,
}

/// Validate that `path` is within one of the allowed directories.
///
/// Allowed directories:
/// - `workspace_dir` — the agent's working directory
/// - `media_store_dir` — the media storage root
///
/// Returns `Ok(path)` if valid, `Err(AdapterError)` if out of bounds.
pub(crate) async fn validate_outbound_path(
    path: &Path,
    workspace_dir: Option<&Path>,
    media_store_dir: &Path,
) -> Result<PathBuf, AdapterError> {
    // Canonicalize the path to resolve `..` components and symlinks.
    // This is a fast syscall and safe to call in async context.
    let canonical = path.canonicalize().map_err(|e| {
        tracing::warn!(path = %path.display(), error = %e, "outbound path canonicalize failed");
        AdapterError::SendFailed(format!("cannot resolve path: {e}"))
    })?;

    let media_canonical = media_store_dir
        .canonicalize()
        .unwrap_or_else(|_| media_store_dir.to_path_buf());
    if canonical.starts_with(&media_canonical) {
        return Ok(canonical);
    }

    if let Some(workspace) = workspace_dir {
        let ws_canonical = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        if canonical.starts_with(&ws_canonical) {
            return Ok(canonical);
        }
    }

    tracing::warn!(
        path = %path.display(),
        "outbound media path rejected: outside allowed directories"
    );
    Err(AdapterError::SendFailed(format!(
        "media path rejected: {} is outside allowed directories",
        path.display()
    )))
}

/// Prepare outbound local media: resolve path, validate, and copy to outbound directory.
///
/// Standalone function (no dependency on `FeishuPlugin`) for reuse across all
/// send paths including the card media fallback route.
///
/// # Arguments
/// * `reference` - Media reference string (path or URL)
/// * `workspace_dir` - Agent workspace directory for resolving relative paths
/// * `media_store` - Media store for outbound directory and whitelist validation
///
/// Returns `Some(outbound_path)` when a local file was successfully validated
/// and copied to the outbound directory. Returns `None` when:
/// - `reference` is an HTTP(S) URL (already platform-hosted)
/// - File does not exist
/// - Path fails whitelist validation
/// - Copy to outbound fails (warning logged)
pub(crate) async fn prepare_outbound_local_media(
    reference: &str,
    workspace_dir: Option<&Path>,
    media_store: &MediaStore,
) -> Option<PathBuf> {
    // HTTP(S) URLs are already platform-hosted — no local copy needed.
    if reference.starts_with("http://") || reference.starts_with("https://") {
        return None;
    }

    let raw_path = PathBuf::from(reference);

    // Resolve relative paths against workspace_dir.
    let path = if raw_path.is_relative() {
        if let Some(ws) = workspace_dir {
            ws.join(&raw_path)
        } else {
            raw_path
        }
    } else {
        raw_path
    };

    if !path.exists() {
        return None;
    }

    // Validate against whitelist (media store + workspace).
    let media_dir = media_store.storage_dir();
    let validated = validate_outbound_path(&path, workspace_dir, media_dir)
        .await
        .ok()?;

    match copy_to_outbound(&validated, media_store).await {
        Ok(result) => Some(result.outbound_path),
        Err(e) => {
            warn!(source = %validated.display(), error = %e, "Failed to copy media to outbound");
            None
        }
    }
}

/// Copy a media file to the outbound directory and return the result.
pub(crate) async fn copy_to_outbound(
    source_path: &Path,
    media_store: &MediaStore,
) -> Result<OutboundMediaResult, AdapterError> {
    let filename = source_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "media".to_string());

    let outbound_dir = media_store.outbound_dir();
    let safe_name = crate::media_store::sanitize_filename(&filename);
    let stem = Path::new(&safe_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "media".to_string());
    let ext = Path::new(&safe_name)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let unique_name = crate::media_store::unique_filename(outbound_dir, &stem, &ext);
    let outbound_path = outbound_dir.join(&unique_name);

    // Ensure outbound directory exists (may not have been created by MediaStore::new
    // in all code paths, e.g. when using a shared media store).
    let _ = tokio::fs::create_dir_all(outbound_dir).await;
    tokio::fs::copy(source_path, &outbound_path)
        .await
        .map_err(|e| {
            tracing::warn!(
                source = %source_path.display(),
                dest = %outbound_path.display(),
                error = %e,
                "failed to copy media to outbound directory"
            );
            AdapterError::SendFailed(format!("failed to copy media: {e}"))
        })?;

    Ok(OutboundMediaResult { outbound_path })
}
/// Upload an image file to Feishu via lark-cli and return the image key.
pub(crate) async fn upload_image(
    adapter: &FeishuAdapter,
    file_path: &Path,
) -> Result<String, AdapterError> {
    let metadata = tokio::fs::metadata(file_path)
        .await
        .map_err(|e| AdapterError::SendFailed(format!("cannot read image metadata: {e}")))?;
    if metadata.len() > MAX_IMAGE_SIZE {
        return Err(AdapterError::SendFailed(format!(
            "image too large: {} bytes (max {})",
            metadata.len(),
            MAX_IMAGE_SIZE
        )));
    }
    let path_str = file_path
        .to_str()
        .ok_or_else(|| AdapterError::SendFailed("image path is not valid UTF-8".to_string()))?;
    let output =
        super::send_helpers::run_cli(adapter, &["im", "+images-upload", "--image", path_str])
            .await?;

    let resp: serde_json::Value = serde_json::from_str(&output).map_err(|e| {
        AdapterError::InvalidPayload(format!("lark-cli images-upload invalid JSON: {e}"))
    })?;
    let code = resp.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = resp
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown");
        return Err(AdapterError::SendFailed(format!(
            "Feishu image upload error {code}: {msg}"
        )));
    }
    resp.get("data")
        .and_then(|d| d.get("image_key"))
        .and_then(|k| k.as_str())
        .map(String::from)
        .ok_or_else(|| {
            AdapterError::SendFailed("image upload response missing image_key".to_string())
        })
}

/// Upload a file to Feishu via lark-cli and return the file key.
pub(crate) async fn upload_file(
    adapter: &FeishuAdapter,
    file_path: &Path,
    filename: &str,
) -> Result<String, AdapterError> {
    validate_file_size(file_path, MAX_FILE_SIZE).await?;
    let path_str = file_path
        .to_str()
        .ok_or_else(|| AdapterError::SendFailed("file path is not valid UTF-8".to_string()))?;
    let file_type = detect_file_type(file_path);
    let output = super::send_helpers::run_cli(
        adapter,
        &[
            "im",
            "+files-upload",
            "--file",
            path_str,
            "--file-type",
            file_type,
            "--file-name",
            filename,
        ],
    )
    .await?;
    parse_file_upload_response(&output)
}

/// Validate that a file does not exceed `max_size` bytes.
async fn validate_file_size(file_path: &Path, max_size: u64) -> Result<(), AdapterError> {
    let metadata = tokio::fs::metadata(file_path)
        .await
        .map_err(|e| AdapterError::SendFailed(format!("cannot read file metadata: {e}")))?;
    if metadata.len() > max_size {
        return Err(AdapterError::SendFailed(format!(
            "file too large: {} bytes (max {})",
            metadata.len(),
            max_size
        )));
    }
    Ok(())
}

/// Parse the JSON response from `lark-cli +files-upload` and extract the file key.
fn parse_file_upload_response(output: &str) -> Result<String, AdapterError> {
    let resp: serde_json::Value = serde_json::from_str(output).map_err(|e| {
        AdapterError::InvalidPayload(format!("lark-cli files-upload invalid JSON: {e}"))
    })?;
    let code = resp.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = resp
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown");
        return Err(AdapterError::SendFailed(format!(
            "Feishu file upload error {code}: {msg}"
        )));
    }
    resp.get("data")
        .and_then(|d| d.get("file_key"))
        .and_then(|k| k.as_str())
        .map(String::from)
        .ok_or_else(|| {
            AdapterError::SendFailed("file upload response missing file_key".to_string())
        })
}

/// Detect Feishu file type from extension.
pub(crate) fn detect_file_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "opus" => "opus",
        "mp4" => "mp4",
        "pdf" => "pdf",
        "doc" | "docx" => "doc",
        "xls" | "xlsx" => "xls",
        "ppt" | "pptx" => "ppt",
        _ => "stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn validate_path_within_workspace() {
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
    async fn validate_path_within_media_store() {
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
    async fn validate_path_outside_rejected() {
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
    async fn validate_path_no_workspace_only_media() {
        let tmp = TempDir::new().unwrap();
        let media = tmp.path().join("media");
        fs::create_dir_all(&media).unwrap();
        let file = media.join("test.txt");
        fs::write(&file, "content").unwrap();
        let result = validate_outbound_path(&file, None, &media).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn copy_to_outbound_creates_file() {
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
    async fn copy_to_outbound_unique_name_on_conflict() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("test.png");
        fs::write(&source, "data1").unwrap();
        let media_store = MediaStore::new(tmp.path().to_str().unwrap()).unwrap();
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
    fn detect_file_type_opus() {
        assert_eq!(detect_file_type(Path::new("audio.opus")), "opus");
    }

    #[test]
    fn detect_file_type_mp4() {
        assert_eq!(detect_file_type(Path::new("video.mp4")), "mp4");
    }

    #[test]
    fn detect_file_type_pdf() {
        assert_eq!(detect_file_type(Path::new("doc.pdf")), "pdf");
    }

    #[test]
    fn detect_file_type_unknown() {
        assert_eq!(detect_file_type(Path::new("data.xyz")), "stream");
    }
}
