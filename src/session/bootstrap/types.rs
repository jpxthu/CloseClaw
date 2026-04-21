//! Bootstrap types — errors, region structs, and constants

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Errors that can occur during bootstrap protection operations
#[derive(Error, Debug)]
pub enum BootstrapProtectionError {
    #[error("bootstrap file not found: {0}")]
    FileNotFound(String),

    #[error("bootstrap content integrity check failed for region: {0}")]
    IntegrityCheckFailed(String),

    #[error("failed to read bootstrap file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("failed to parse region marker: {0}")]
    MarkerParseError(String),

    #[error("workspace path required for reinject")]
    WorkspacePathRequired,
}

/// Bootstrap region markers — placed in transcript to delimit bootstrap content
///
/// These markers wrap the bootstrap content in the session transcript, allowing
/// us to identify and verify the bootstrap content before/after compaction.
pub const BOOTSTRAP_REGION_START: &str = "<bootstrap:";
pub const BOOTSTRAP_REGION_END: &str = "</bootstrap>";

/// Bootstrap region — metadata for one bootstrap file's region in the transcript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapRegion {
    /// Unique region identifier (session-scoped)
    pub region_id: String,
    /// Name of the bootstrap file (e.g., "AGENTS.md")
    pub file_name: String,
    /// SHA-256 hash of the original file content (for integrity check)
    pub content_hash: String,
    /// Character count of the original content
    pub char_count: usize,
    /// Whether this is the original injection or a re-injection after compaction
    pub is_reinject: bool,
    /// Original injection timestamp
    pub injected_at: DateTime<Utc>,
    /// Character offset where this region starts in the transcript (if known)
    pub transcript_offset: Option<usize>,
}

impl BootstrapRegion {
    /// Create a new BootstrapRegion from a file
    pub fn new(file_name: impl Into<String>, content: &str, is_reinject: bool) -> Self {
        let file_name = file_name.into();
        let char_count = content.chars().count();
        let full_hash = Self::compute_hash(content);
        // Store first 12 hex chars for marker compactness
        let content_hash = full_hash[..12].to_string();
        let region_id = format!("{}_{}", file_name, content_hash[..8].to_string());

        Self {
            region_id,
            file_name,
            content_hash,
            char_count,
            is_reinject,
            injected_at: Utc::now(),
            transcript_offset: None,
        }
    }

    /// Compute SHA-256 hash of content (hex string)
    pub fn compute_hash(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }

    /// Verify content integrity against stored hash.
    /// Compares the first 12 hex chars of the computed hash with stored 12-char hash.
    pub fn verify_integrity(&self, content: &str) -> bool {
        let full_hash = Self::compute_hash(content);
        let stored_len = self.content_hash.chars().count();
        // Compare the prefix that matches what we stored (12 chars)
        full_hash[..stored_len.min(64)] == self.content_hash
    }

    /// Generate the start marker string for this region
    pub fn start_marker(&self) -> String {
        format!(
            "{}file={},hash={},chars={},reinject={}>",
            BOOTSTRAP_REGION_START,
            self.file_name,
            self.content_hash,
            self.char_count,
            self.is_reinject
        )
    }

    /// Generate the end marker string
    pub fn end_marker(&self) -> String {
        BOOTSTRAP_REGION_END.to_string()
    }

    /// Wrap content with markers
    pub fn wrap_content(&self, content: &str) -> String {
        format!(
            "{}\n{}\n{}",
            self.start_marker(),
            content,
            self.end_marker()
        )
    }

    /// Parse a region from a marker string and content
    pub fn parse_from_marker(
        marker: &str,
        content: &str,
        injected_at: DateTime<Utc>,
    ) -> Result<Self, BootstrapProtectionError> {
        // marker looks like: <bootstrap:file=X,hash=Y,chars=Z,reinject=W>
        let marker_stripped = marker
            .strip_prefix(BOOTSTRAP_REGION_START)
            .ok_or_else(|| BootstrapProtectionError::MarkerParseError(marker.to_string()))?
            .trim_end_matches('>');

        let mut file_name = None;
        let mut hash = None;
        let mut chars = None;
        let mut reinject = None;

        for part in marker_stripped.split(',') {
            let mut iter = part.splitn(2, '=');
            let key = iter.next().map(|s| s.trim());
            let value = iter.next().map(|s| s.trim());

            match key {
                Some("file") => file_name = value.map(String::from),
                Some("hash") => hash = value.map(String::from),
                Some("chars") => chars = value.and_then(|s| s.parse().ok()),
                Some("reinject") => reinject = value.map(|s| s == "true"),
                _ => {}
            }
        }

        let file_name = file_name.ok_or_else(|| {
            BootstrapProtectionError::MarkerParseError(format!(
                "missing file in marker: {}",
                marker
            ))
        })?;
        let hash = hash.ok_or_else(|| {
            BootstrapProtectionError::MarkerParseError(format!(
                "missing hash in marker: {}",
                marker
            ))
        })?;
        let char_count = chars.unwrap_or_else(|| content.chars().count());
        let is_reinject = reinject.unwrap_or(false);

        let region_id = format!("{}_{}", file_name, &hash[..8]);

        Ok(Self {
            region_id,
            file_name,
            content_hash: hash,
            char_count,
            is_reinject,
            injected_at,
            transcript_offset: None,
        })
    }
}
