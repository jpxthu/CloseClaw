//! Media reference resolution for tool execution.
//!
//! Scans tool arguments for `[type: key]` reference tokens and resolves
//! them to local file paths via [`MediaStoreAccess`](closeclaw_common::MediaStoreAccess).
//!
//! Resolution only applies to the current tool execution — translated
//! paths are not persisted to params or logs.

use std::sync::LazyLock;

use closeclaw_common::MediaStoreAccess;
use closeclaw_common::MediaStoreError;
use regex::Regex;
use serde_json::Value;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from media reference resolution.
#[derive(Debug, Error)]
pub enum MediaRefError {
    /// Media store is not available for resolution.
    #[error("media store not available")]
    StoreUnavailable,

    /// Reference token could not be resolved.
    #[error("media reference not found: [{media_type}: {key}]")]
    NotFound { media_type: String, key: String },

    /// Media store returned an error.
    #[error("media store error: {0}")]
    Store(#[from] MediaStoreError),
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Regex matching media reference tokens: `[image: key]`, `[file: key]`, `[audio: key]`.
///
/// Captures:
/// 1. Media type label (`image`, `file`, `audio`)
/// 2. Media key (trimmed whitespace)
static MEDIA_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[(image|file|audio):\s*([^\]]+)\]").expect("valid regex"));

/// Resolve media reference tokens in a JSON value.
///
/// Recursively scans `args` for string values containing `[type: key]`
/// patterns and replaces them with the resolved local file path.
///
/// Resolution is **transient**: the original `args` is not modified;
/// a new `Value` is returned with paths substituted.
///
/// # Arguments
///
/// * `args` — Tool arguments to scan.
/// * `media_store` — Store for resolving references to local paths.
///
/// # Returns
///
/// A new `Value` with reference tokens replaced by resolved paths,
/// or `Err` if any reference cannot be resolved.
pub fn resolve_media_refs(
    args: &Value,
    media_store: &dyn MediaStoreAccess,
) -> Result<Value, MediaRefError> {
    let store = media_store;
    resolve_value(args, store)
}

/// Recursively resolve media references in a JSON value.
fn resolve_value(value: &Value, store: &dyn MediaStoreAccess) -> Result<Value, MediaRefError> {
    match value {
        Value::String(s) => resolve_string(s, store).map(Value::String),
        Value::Array(arr) => {
            let resolved: Result<Vec<Value>, _> =
                arr.iter().map(|v| resolve_value(v, store)).collect();
            resolved.map(Value::Array)
        }
        Value::Object(map) => {
            let mut resolved = serde_json::Map::new();
            for (k, v) in map {
                resolved.insert(k.clone(), resolve_value(v, store)?);
            }
            Ok(Value::Object(resolved))
        }
        // Numbers, bools, null — pass through unchanged.
        other => Ok(other.clone()),
    }
}

/// Resolve media references in a single string.
///
/// If the string contains `[type: key]` tokens, each is resolved
/// and the string is returned with paths substituted.
fn resolve_string(s: &str, store: &dyn MediaStoreAccess) -> Result<String, MediaRefError> {
    if !s.contains('[') {
        return Ok(s.to_string());
    }

    let mut result = String::with_capacity(s.len());
    let mut last_end = 0;

    for cap in MEDIA_REF_RE.captures_iter(s) {
        let m = cap.get(0).unwrap();
        let media_type = cap[1].to_string();
        let key = cap[2].trim().to_string();

        // Append text before this match.
        result.push_str(&s[last_end..m.start()]);

        // Resolve the reference.
        let media_ref = closeclaw_common::MediaRef {
            key: key.clone(),
            path: String::new(),
            media_type: match media_type.as_str() {
                "image" => closeclaw_common::MediaType::Image,
                "file" => closeclaw_common::MediaType::File,
                "audio" => closeclaw_common::MediaType::Audio,
                _ => return Err(MediaRefError::NotFound { media_type, key }),
            },
            size: 0,
            mime: String::new(),
        };

        match store.resolve_ref(&media_ref) {
            Ok(path) => {
                result.push_str(&path.to_string_lossy());
            }
            Err(MediaStoreError::NoPath) | Err(MediaStoreError::FileNotFound(_)) => {
                return Err(MediaRefError::NotFound { media_type, key });
            }
            Err(e) => return Err(MediaRefError::Store(e)),
        }

        last_end = m.end();
    }

    result.push_str(&s[last_end..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Simple file-system-backed store for testing.
    struct FsStore {
        root: std::path::PathBuf,
        refs: std::collections::HashMap<String, std::path::PathBuf>,
    }

    impl FsStore {
        fn new(root: std::path::PathBuf) -> Self {
            Self {
                root,
                refs: std::collections::HashMap::new(),
            }
        }

        fn add_ref(&mut self, key: &str, path: &str) {
            self.refs.insert(key.to_string(), self.root.join(path));
        }
    }

    impl closeclaw_common::MediaStoreAccess for FsStore {
        fn resolve_ref(
            &self,
            media_ref: &closeclaw_common::MediaRef,
        ) -> Result<PathBuf, MediaStoreError> {
            if media_ref.path.is_empty() {
                if let Some(path) = self.refs.get(&media_ref.key) {
                    if path.exists() {
                        return Ok(path.clone());
                    }
                    return Err(MediaStoreError::FileNotFound(path.clone()));
                }
                return Err(MediaStoreError::NoPath);
            }
            let full = self.root.join(&media_ref.path);
            if !full.exists() {
                return Err(MediaStoreError::FileNotFound(full));
            }
            Ok(full)
        }
    }

    #[test]
    fn resolve_no_refs_passthrough() {
        let tmp = TempDir::new().unwrap();
        let store = FsStore::new(tmp.path().to_path_buf());
        let args = serde_json::json!({"path": "/some/file.txt"});
        let resolved = resolve_media_refs(&args, &store).unwrap();
        assert_eq!(resolved, args);
    }

    #[test]
    fn resolve_string_with_ref() {
        let tmp = TempDir::new().unwrap();
        let mut store = FsStore::new(tmp.path().to_path_buf());
        store.add_ref("img_001", "inbound/img.png");
        // Create the file so it exists.
        std::fs::create_dir_all(tmp.path().join("inbound")).unwrap();
        std::fs::write(tmp.path().join("inbound/img.png"), "data").unwrap();

        let args = serde_json::json!({"path": "[image: img_001]"});
        let resolved = resolve_media_refs(&args, &store).unwrap();
        let expected = tmp
            .path()
            .join("inbound/img.png")
            .to_string_lossy()
            .to_string();
        assert_eq!(resolved["path"], expected);
    }

    #[test]
    fn resolve_missing_ref_errors() {
        let tmp = TempDir::new().unwrap();
        let store = FsStore::new(tmp.path().to_path_buf());
        let args = serde_json::json!({"path": "[image: nonexistent]"});
        let err = resolve_media_refs(&args, &store).unwrap_err();
        assert!(matches!(err, MediaRefError::NotFound { .. }));
    }

    #[test]
    fn resolve_multiple_refs() {
        let tmp = TempDir::new().unwrap();
        let mut store = FsStore::new(tmp.path().to_path_buf());
        store.add_ref("a", "a.png");
        store.add_ref("b", "b.pdf");
        std::fs::write(tmp.path().join("a.png"), "").unwrap();
        std::fs::write(tmp.path().join("b.pdf"), "").unwrap();

        let args = serde_json::json!({
            "images": "[image: a] and [file: b]"
        });
        let resolved = resolve_media_refs(&args, &store).unwrap();
        let expected_a = tmp.path().join("a.png").to_string_lossy().to_string();
        let expected_b = tmp.path().join("b.pdf").to_string_lossy().to_string();
        assert!(resolved["images"].as_str().unwrap().contains(&expected_a));
        assert!(resolved["images"].as_str().unwrap().contains(&expected_b));
    }

    #[test]
    fn resolve_nested_objects() {
        let tmp = TempDir::new().unwrap();
        let mut store = FsStore::new(tmp.path().to_path_buf());
        store.add_ref("k", "file.txt");
        std::fs::write(tmp.path().join("file.txt"), "").unwrap();

        let args = serde_json::json!({
            "outer": {
                "inner": "[file: k]"
            }
        });
        let resolved = resolve_media_refs(&args, &store).unwrap();
        let expected = tmp.path().join("file.txt").to_string_lossy().to_string();
        assert_eq!(resolved["outer"]["inner"], expected);
    }

    #[test]
    fn resolve_array_elements() {
        let tmp = TempDir::new().unwrap();
        let mut store = FsStore::new(tmp.path().to_path_buf());
        store.add_ref("x", "x.png");
        std::fs::write(tmp.path().join("x.png"), "").unwrap();

        let args = serde_json::json!({
            "items": ["[image: x]", "plain text"]
        });
        let resolved = resolve_media_refs(&args, &store).unwrap();
        let expected = tmp.path().join("x.png").to_string_lossy().to_string();
        assert_eq!(resolved["items"][0], expected);
        assert_eq!(resolved["items"][1], "plain text");
    }

    #[test]
    fn resolve_no_store_errors() {
        // A store that always returns NoPath.
        struct NoStore;
        impl closeclaw_common::MediaStoreAccess for NoStore {
            fn resolve_ref(
                &self,
                _: &closeclaw_common::MediaRef,
            ) -> Result<PathBuf, MediaStoreError> {
                Err(MediaStoreError::NoPath)
            }
        }

        let store = NoStore;
        let args = serde_json::json!({"path": "[image: k]"});
        let err = resolve_media_refs(&args, &store).unwrap_err();
        assert!(matches!(err, MediaRefError::NotFound { .. }));
    }

    #[test]
    fn resolve_non_string_values_unchanged() {
        let tmp = TempDir::new().unwrap();
        let store = FsStore::new(tmp.path().to_path_buf());
        let args = serde_json::json!({
            "count": 42,
            "flag": true,
            "data": null
        });
        let resolved = resolve_media_refs(&args, &store).unwrap();
        assert_eq!(resolved["count"], 42);
        assert_eq!(resolved["flag"], true);
        assert!(resolved["data"].is_null());
    }
}
