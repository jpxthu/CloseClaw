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
    let path_owned = path.to_path_buf();
    let canonical = tokio::task::spawn_blocking(move || path_owned.canonicalize())
        .await
        .map_err(|e| AdapterError::SendFailed(format!("task join error: {e}")))?
        .map_err(|e| {
            tracing::warn!(path = %path.display(), error = %e, "outbound path canonicalize failed");
            AdapterError::SendFailed(format!("cannot resolve path: {e}"))
        })?;

    let media_owned = media_store_dir.to_path_buf();
    if let Ok(media_canonical) = tokio::task::spawn_blocking(move || media_owned.canonicalize())
        .await
        .map_err(|e| AdapterError::SendFailed(format!("task join error: {e}")))?
    {
        if canonical.starts_with(&media_canonical) {
            return Ok(canonical);
        }
    }

    if let Some(workspace) = workspace_dir {
        let ws_owned = workspace.to_path_buf();
        if let Ok(ws_canonical) = tokio::task::spawn_blocking(move || ws_owned.canonicalize())
            .await
            .map_err(|e| AdapterError::SendFailed(format!("task join error: {e}")))?
        {
            if canonical.starts_with(&ws_canonical) {
                return Ok(canonical);
            }
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

/// Detect MIME type for image upload based on file extension.
fn detect_image_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "image/png",
    }
}

/// Parse an upload response JSON and extract the key for the given field.
fn parse_upload_response(
    resp: serde_json::Value,
    key_field: &str,
    api_name: &str,
) -> Result<String, AdapterError> {
    let code = resp.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = resp
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown");
        tracing::warn!(code = code, msg = %msg, "Feishu {api_name} upload error");
        return Err(AdapterError::SendFailed(format!(
            "Feishu {api_name} upload error {code}: {msg}"
        )));
    }
    resp.get("data")
        .and_then(|d| d.get(key_field))
        .and_then(|k| k.as_str())
        .map(String::from)
        .ok_or_else(|| {
            AdapterError::SendFailed(format!("{api_name} upload response missing {key_field}"))
        })
}

/// Generic multipart upload: send form to Feishu API and parse response.
async fn upload_multipart(
    adapter: &FeishuAdapter,
    api_path: &str,
    form: reqwest::multipart::Form,
    key_field: &str,
    api_name: &str,
) -> Result<String, AdapterError> {
    let token = adapter.get_tenant_token().await.map_err(|e| {
        tracing::warn!(error = %e, "token fetch failed for {api_name} upload");
        e
    })?;
    let url = format!("{}/{}", adapter.base_url, api_path);
    let resp = adapter
        .http_client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "{api_name} upload request failed");
            AdapterError::SendFailed(e.to_string())
        })?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "{api_name} upload response parse failed");
            AdapterError::SendFailed(e.to_string())
        })?;
    parse_upload_response(resp, key_field, api_name)
}

/// Upload an image file to Feishu and return the image key.
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
    let file_bytes = tokio::fs::read(file_path)
        .await
        .map_err(|e| AdapterError::SendFailed(format!("cannot read image file: {e}")))?;
    let filename = file_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "image.png".to_string());
    let mime_type = detect_image_mime(file_path);
    let form = reqwest::multipart::Form::new()
        .part(
            "image_type",
            reqwest::multipart::Part::text("message".to_string()),
        )
        .part(
            "image",
            reqwest::multipart::Part::bytes(file_bytes)
                .file_name(filename)
                .mime_str(mime_type)
                .map_err(|e| AdapterError::SendFailed(e.to_string()))?,
        );
    upload_multipart(adapter, "im/v1/images", form, "image_key", "image").await
}

/// Upload a file to Feishu and return the file key.
pub(crate) async fn upload_file(
    adapter: &FeishuAdapter,
    file_path: &Path,
    filename: &str,
) -> Result<String, AdapterError> {
    let metadata = tokio::fs::metadata(file_path)
        .await
        .map_err(|e| AdapterError::SendFailed(format!("cannot read file metadata: {e}")))?;
    if metadata.len() > MAX_FILE_SIZE {
        return Err(AdapterError::SendFailed(format!(
            "file too large: {} bytes (max {})",
            metadata.len(),
            MAX_FILE_SIZE
        )));
    }
    let file_bytes = tokio::fs::read(file_path)
        .await
        .map_err(|e| AdapterError::SendFailed(format!("cannot read file: {e}")))?;
    let file_type = detect_file_type(file_path);
    let form = reqwest::multipart::Form::new()
        .part(
            "file_type",
            reqwest::multipart::Part::text(file_type.to_string()),
        )
        .part(
            "file_name",
            reqwest::multipart::Part::text(filename.to_string()),
        )
        .part(
            "file",
            reqwest::multipart::Part::bytes(file_bytes)
                .file_name(filename.to_string())
                .mime_str("application/octet-stream")
                .map_err(|e| AdapterError::SendFailed(e.to_string()))?,
        );
    upload_multipart(adapter, "im/v1/files", form, "file_key", "file").await
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
