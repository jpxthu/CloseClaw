//! Media Store — local persistence for inbound/outbound media files.
//!
//! Provides download-and-persist, reference resolution, and retention-based
//! cleanup. Used by platform adapters (inbound) and gateway/tools (resolution).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use closeclaw_common::{MediaRef, MediaType};
use thiserror::Error;
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from media storage operations.
#[derive(Debug, Error)]
pub enum MediaStoreError {
    /// I/O error (download, write, read, dir creation).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// HTTP download failure.
    #[error("download failed: {0}")]
    DownloadFailed(String),

    /// File exceeds the configured size limit.
    #[error("file exceeds size limit ({size} bytes > {limit} bytes)")]
    SizeLimitExceeded { size: u64, limit: u64 },

    /// Media reference has no local path to resolve.
    #[error("media reference has no path set")]
    NoPath,

    /// Resolved path does not exist on disk.
    #[error("media file not found: {0}")]
    FileNotFound(PathBuf),
}

// ---------------------------------------------------------------------------
// MediaStore
// ---------------------------------------------------------------------------

/// Local media storage manager.
///
/// Owns the `inbound/` and `outbound/` directories under a configurable
/// storage root and provides methods for download, resolution, and cleanup.
#[derive(Debug, Clone)]
pub struct MediaStore {
    /// Root storage directory.
    storage_dir: PathBuf,
    /// Inbound media sub-directory.
    inbound_dir: PathBuf,
    /// Outbound media sub-directory.
    outbound_dir: PathBuf,
}

impl MediaStore {
    /// Create a new `MediaStore` rooted at `storage_dir`.
    ///
    /// Expands a leading `~` with the user's home directory and creates
    /// both `inbound/` and `outbound/` sub-directories.
    pub fn new(storage_dir: &str) -> Result<Self, MediaStoreError> {
        let expanded = expand_tilde(storage_dir);
        let inbound = expanded.join("inbound");
        let outbound = expanded.join("outbound");
        fs::create_dir_all(&inbound)?;
        fs::create_dir_all(&outbound)?;
        Ok(Self {
            storage_dir: expanded,
            inbound_dir: inbound,
            outbound_dir: outbound,
        })
    }

    /// Download content from `url`, sanitize the filename, write to
    /// `inbound/`, and return a fully-populated [`MediaRef`].
    ///
    /// `max_size_bytes` caps the download; set to `u64::MAX` to disable.
    pub async fn download_and_persist(
        &self,
        url: &str,
        key: &str,
        media_type: &MediaType,
        http_client: &reqwest::Client,
        max_size_bytes: u64,
    ) -> Result<MediaRef, MediaStoreError> {
        let response = http_client
            .get(url)
            .send()
            .await
            .map_err(|e| MediaStoreError::DownloadFailed(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(MediaStoreError::DownloadFailed(format!("HTTP {status}")));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let bytes = response
            .bytes()
            .await
            .map_err(|e| MediaStoreError::DownloadFailed(e.to_string()))?;

        let size = bytes.len() as u64;
        if size > max_size_bytes {
            return Err(MediaStoreError::SizeLimitExceeded {
                size,
                limit: max_size_bytes,
            });
        }

        let extension = mime_to_extension(&content_type);
        let safe_name = sanitize_filename(key);
        let filename = unique_filename(&self.inbound_dir, &safe_name, extension);
        let file_path = self.inbound_dir.join(&filename);

        fs::write(&file_path, &bytes)?;

        let relative_path = format!("inbound/{filename}");
        let mime = content_type;

        Ok(MediaRef {
            key: key.to_string(),
            path: relative_path,
            media_type: media_type.clone(),
            size: size as i64,
            mime,
        })
    }

    /// Resolve a [`MediaRef`] to its absolute local path.
    ///
    /// The `path` field of the ref is treated as relative to the storage
    /// root (e.g. `inbound/file.png`).
    pub fn resolve_ref(&self, media_ref: &MediaRef) -> Result<PathBuf, MediaStoreError> {
        if media_ref.path.is_empty() {
            return Err(MediaStoreError::NoPath);
        }
        let full = self.storage_dir.join(&media_ref.path);
        if !full.exists() {
            return Err(MediaStoreError::FileNotFound(full));
        }
        Ok(full)
    }

    /// Delete files older than `retention_days` in both `inbound/` and
    /// `outbound/`. Returns the number of files removed.
    ///
    /// A `retention_days` of 0 disables cleanup (returns 0 immediately).
    pub fn cleanup_expired(&self, retention_days: u64) -> Result<u64, MediaStoreError> {
        if retention_days == 0 {
            return Ok(0);
        }

        let cutoff = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(retention_days * 86_400))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let mut removed = 0u64;
        removed += cleanup_dir(&self.inbound_dir, cutoff)?;
        removed += cleanup_dir(&self.outbound_dir, cutoff)?;
        Ok(removed)
    }

    /// Return a reference to the inbound directory path.
    pub fn inbound_dir(&self) -> &Path {
        &self.inbound_dir
    }

    /// Return a reference to the outbound directory path.
    pub fn outbound_dir(&self) -> &Path {
        &self.outbound_dir
    }

    /// Return a reference to the storage root path.
    pub fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }
}

impl closeclaw_common::MediaStoreAccess for MediaStore {
    fn resolve_ref(
        &self,
        media_ref: &closeclaw_common::MediaRef,
    ) -> Result<std::path::PathBuf, closeclaw_common::MediaStoreError> {
        if media_ref.path.is_empty() {
            return Err(closeclaw_common::MediaStoreError::NoPath);
        }
        let full = self.storage_dir.join(&media_ref.path);
        if !full.exists() {
            return Err(closeclaw_common::MediaStoreError::FileNotFound(full));
        }
        Ok(full)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Expand a leading `~` with the user's home directory.
///
/// If `dirs::home_dir()` is unavailable or the path doesn't start with `~`,
/// the original string is returned as-is.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix('~')) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Sanitize a filename by removing path separators, control characters, and
/// other unsafe characters while preserving the file extension.
///
/// Empty results fall back to `"media"`.
pub fn sanitize_filename(name: &str) -> String {
    let stem;
    let ext;
    if let Some(dot_pos) = name.rfind('.') {
        stem = &name[..dot_pos];
        ext = &name[dot_pos + 1..];
    } else {
        stem = name;
        ext = "";
    }

    let safe_stem: String = stem
        .chars()
        .filter(|c| !c.is_control() && *c != '/' && *c != '\\' && *c != '\0')
        .collect::<String>()
        .trim()
        .to_string();

    let safe_stem = if safe_stem.is_empty() {
        "media".to_string()
    } else {
        safe_stem
    };

    if ext.is_empty() {
        safe_stem
    } else {
        let safe_ext: String = ext
            .chars()
            .filter(|c| !c.is_control() && *c != '/' && *c != '\\' && *c != '\0')
            .collect();
        format!("{safe_stem}.{safe_ext}")
    }
}

/// Generate a unique filename by appending a short random suffix if a
/// file with the same name already exists.
fn unique_filename(dir: &Path, name: &str, extension: &str) -> String {
    let base = if extension.is_empty() {
        name.to_string()
    } else {
        format!("{name}.{extension}")
    };

    if !dir.join(&base).exists() {
        return base;
    }

    for _ in 0..100 {
        let suffix = &uuid::Uuid::new_v4().to_string()[..8];
        let candidate = if extension.is_empty() {
            format!("{name}_{suffix}")
        } else {
            format!("{name}_{suffix}.{extension}")
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }

    // Extremely unlikely — fall back to full UUID.
    let uuid_str = uuid::Uuid::new_v4().to_string();
    if extension.is_empty() {
        format!("{name}_{uuid_str}")
    } else {
        format!("{name}_{uuid_str}.{extension}")
    }
}

/// Map a MIME type to a file extension.
fn mime_to_extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/wav" => "wav",
        "video/mp4" => "mp4",
        "application/zip" => "zip",
        "application/json" => "json",
        "text/plain" => "txt",
        "text/markdown" => "md",
        _ => "bin",
    }
}

/// Delete files in `dir` whose last-modified time is before `cutoff`.
fn cleanup_dir(dir: &Path, cutoff: SystemTime) -> Result<u64, MediaStoreError> {
    let mut removed = 0u64;
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };

    for entry in entries.flatten() {
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                warn!(path = %entry.path().display(), error = %e, "failed to read metadata");
                continue;
            }
        };

        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        if modified < cutoff {
            match fs::remove_file(entry.path()) {
                Ok(()) => {
                    debug!(path = %entry.path().display(), "removed expired media file");
                    removed += 1;
                }
                Err(e) => {
                    warn!(path = %entry.path().display(), error = %e, "failed to remove expired file");
                }
            }
        }
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a MediaStore rooted in a temp directory.
    fn make_store() -> (TempDir, MediaStore) {
        let tmp = TempDir::new().expect("tmp dir");
        let store = MediaStore::new(tmp.path().to_str().unwrap()).expect("media store creation");
        (tmp, store)
    }

    // -- sanitize_filename tests --

    #[test]
    fn sanitize_removes_path_separators() {
        assert_eq!(sanitize_filename("foo/bar.png"), "foobar.png");
        assert_eq!(sanitize_filename("foo\\bar.png"), "foobar.png");
    }

    #[test]
    fn sanitize_removes_control_chars() {
        assert_eq!(sanitize_filename("hello\x00world.png"), "helloworld.png");
        assert_eq!(sanitize_filename("a\nb\r.png"), "ab.png");
    }

    #[test]
    fn sanitize_empty_becomes_media() {
        assert_eq!(sanitize_filename(""), "media");
        assert_eq!(sanitize_filename("   "), "media");
        assert_eq!(sanitize_filename("/\\\x00"), "media");
    }

    #[test]
    fn sanitize_preserves_extension() {
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("archive.tar.gz"), "archive.tar.gz");
    }

    #[test]
    fn sanitize_long_name() {
        let long = "a".repeat(500);
        let result = sanitize_filename(&format!("{long}.png"));
        assert!(result.ends_with(".png"));
        assert!(result.len() <= 504);
    }

    // -- new() tests --

    #[test]
    fn new_creates_inbound_and_outbound() {
        let (tmp, store) = make_store();
        assert!(store.inbound_dir().exists());
        assert!(store.outbound_dir().exists());
        assert_eq!(store.storage_dir(), tmp.path());
    }

    // -- resolve_ref tests --

    #[test]
    fn resolve_ref_returns_path_when_file_exists() {
        let (_tmp, store) = make_store();
        let file_path = store.inbound_dir().join("test.txt");
        fs::write(&file_path, "content").unwrap();

        let media_ref = MediaRef {
            key: "k".into(),
            path: "inbound/test.txt".into(),
            media_type: MediaType::File,
            size: 7,
            mime: "text/plain".into(),
        };

        let resolved = store.resolve_ref(&media_ref).unwrap();
        assert_eq!(resolved, file_path);
    }

    #[test]
    fn resolve_ref_errors_on_empty_path() {
        let (_tmp, store) = make_store();
        let media_ref = MediaRef {
            key: "k".into(),
            path: String::new(),
            media_type: MediaType::File,
            size: 0,
            mime: "text/plain".into(),
        };
        assert!(matches!(
            store.resolve_ref(&media_ref),
            Err(MediaStoreError::NoPath)
        ));
    }

    #[test]
    fn resolve_ref_errors_when_file_missing() {
        let (_tmp, store) = make_store();
        let media_ref = MediaRef {
            key: "k".into(),
            path: "inbound/nonexistent.png".into(),
            media_type: MediaType::Image,
            size: 0,
            mime: "image/png".into(),
        };
        assert!(matches!(
            store.resolve_ref(&media_ref),
            Err(MediaStoreError::FileNotFound(_))
        ));
    }

    // -- cleanup tests --

    #[test]
    fn cleanup_expired_disabled_when_zero() {
        let (_tmp, store) = make_store();
        let removed = store.cleanup_expired(0).unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn cleanup_removes_old_files() {
        let (_tmp, store) = make_store();
        let file_path = store.inbound_dir().join("old.txt");
        fs::write(&file_path, "old content").unwrap();

        // Set modified time to 10 days ago.
        let ten_days_ago = SystemTime::now() - std::time::Duration::from_secs(10 * 86_400);
        let file = fs::File::open(&file_path).expect("open file");
        file.set_times(
            fs::FileTimes::new()
                .set_modified(ten_days_ago)
                .set_accessed(ten_days_ago),
        )
        .expect("set file time");

        let removed = store.cleanup_expired(7).unwrap();
        assert_eq!(removed, 1);
        assert!(!file_path.exists());
    }

    #[test]
    fn cleanup_keeps_recent_files() {
        let (_tmp, store) = make_store();
        let file_path = store.inbound_dir().join("recent.txt");
        fs::write(&file_path, "recent content").unwrap();

        let removed = store.cleanup_expired(7).unwrap();
        assert_eq!(removed, 0);
        assert!(file_path.exists());
    }

    // -- unique_filename tests --

    #[test]
    fn unique_filename_no_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_filename(dir.path(), "photo", "png");
        assert_eq!(name, "photo.png");
    }

    #[test]
    fn unique_filename_with_conflict() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("photo.png"), "").unwrap();
        let name = unique_filename(dir.path(), "photo", "png");
        assert!(name.starts_with("photo_"));
        assert!(name.ends_with(".png"));
        assert_ne!(name, "photo.png");
    }

    // -- expand_tilde tests --

    #[test]
    fn expand_tilde_with_home() {
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(expand_tilde("~/foo"), home.join("foo"));
        assert_eq!(expand_tilde("~"), home);
    }

    #[test]
    fn expand_tilde_no_home_prefix() {
        assert_eq!(
            expand_tilde("/absolute/path"),
            PathBuf::from("/absolute/path")
        );
        assert_eq!(
            expand_tilde("relative/path"),
            PathBuf::from("relative/path")
        );
    }
}
