//! Bootstrap Protection Layer — Compaction 防护机制
//!
//! Ensures that agent bootstrap files (AGENTS.md, SOUL.md, IDENTITY.md, USER.md)
//! are not summarization-distorted during OpenClaw session compaction.
//!
//! # Core Concept
//!
//! When OpenClaw triggers compaction on a long session, the bootstrap context
//! (injected at session start) gets summarized along with the transcript history.
//! This module provides:
//!
//! - [`BootstrapRegion`] — Marker structs delimiting bootstrap content in transcript
//! - [`BootstrapContext`] — Metadata tracking all bootstrap regions and their integrity
//! - [`BootstrapProtection`] — Main service for protecting/re-injecting bootstrap content
//!
//! # Usage
//!
//! 1. At session start: [`BootstrapProtection::protect_session`] to scan and mark bootstrap content
//! 2. Before compaction: [`BootstrapProtection::before_compact`] to store integrity hashes
//! 3. After compaction: [`BootstrapProtection::after_compact`] to detect corruption
//! 4. If corrupted: [`BootstrapProtection::reinject`] to prepend fresh bootstrap content

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
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

/// Bootstrap context metadata — stored alongside session state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapContext {
    /// All bootstrap regions currently tracked in the transcript
    pub regions: Vec<BootstrapRegion>,
    /// Whether bootstrap has been re-injected after the last compaction
    pub reinjected_after_last_compact: bool,
    /// Total character count of all bootstrap content
    pub total_char_count: usize,
    /// Integrity hashes stored at last before_compact call (keyed by region_id)
    #[serde(default)]
    pub pre_compact_hashes: HashMap<String, String>,
}

impl Default for BootstrapContext {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
            // At session start, bootstrap is in its "re-injected" (original) state
            reinjected_after_last_compact: true,
            total_char_count: 0,
            pre_compact_hashes: HashMap::new(),
        }
    }
}

impl BootstrapContext {
    /// Update total character count from regions
    pub fn update_total_char_count(&mut self) {
        self.total_char_count = self.regions.iter().map(|r| r.char_count).sum();
    }

    /// Check if any region has had its content corrupted (hash mismatch)
    pub fn check_integrity<'a>(
        &self,
        contents: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Vec<String> {
        // contents: iterator of (file_name, content)
        let mut corrupted = Vec::new();
        for (file_name, content) in contents {
            if let Some(region) = self.regions.iter().find(|r| r.file_name == file_name) {
                if !region.verify_integrity(content) {
                    corrupted.push(file_name.to_string());
                }
            }
        }
        corrupted
    }

    /// Add a new region and update total char count
    pub fn add_region(&mut self, region: BootstrapRegion) {
        self.total_char_count += region.char_count;
        self.regions.push(region);
    }

    /// Mark all regions as reinjected
    pub fn mark_all_reinjected(&mut self) {
        for region in &mut self.regions {
            region.is_reinject = true;
        }
        self.reinjected_after_last_compact = true;
    }

    /// Check if total bootstrap size exceeds limit
    pub fn exceeds_size_limit(&self, limit: usize) -> bool {
        self.total_char_count > limit
    }
}

/// Bootstrap protection service
#[derive(Debug, Clone)]
pub struct BootstrapProtection {
    /// Path to the agent workspace (contains AGENTS.md, SOUL.md, etc.)
    workspace_path: Option<PathBuf>,
    /// Bootstrap files to protect (in order of prepend)
    bootstrap_files: Vec<String>,
    /// Size limit per reinject (default 60K chars)
    size_limit: usize,
}

impl Default for BootstrapProtection {
    fn default() -> Self {
        Self::new()
    }
}

impl BootstrapProtection {
    /// Create a new BootstrapProtection with default settings
    pub fn new() -> Self {
        Self {
            workspace_path: None,
            bootstrap_files: vec![
                "AGENTS.md".to_string(),
                "SOUL.md".to_string(),
                "IDENTITY.md".to_string(),
                "USER.md".to_string(),
            ],
            size_limit: 60 * 1024, // 60K chars
        }
    }

    /// Create with a specific workspace path
    pub fn with_workspace(mut self, path: PathBuf) -> Self {
        self.workspace_path = Some(path);
        self
    }

    /// Set custom bootstrap file list
    pub fn with_bootstrap_files(mut self, files: Vec<String>) -> Self {
        self.bootstrap_files = files;
        self
    }

    /// Set the size limit for reinject (in characters)
    pub fn with_size_limit(mut self, limit: usize) -> Self {
        self.size_limit = limit;
        self
    }

    /// Protect a session by scanning the transcript for existing bootstrap injection
    /// and wrapping it with region markers.
    ///
    /// Returns the modified transcript and the initial BootstrapContext.
    pub fn protect_session(&self, transcript: &str) -> (String, BootstrapContext) {
        let mut regions = Vec::new();
        let mut modified = transcript.to_string();

        // Try to find bootstrap content in the transcript using heuristic markers
        // We look for the start of known bootstrap file content
        for file_name in &self.bootstrap_files {
            if let Some(content) = self.find_bootstrap_content(transcript, file_name) {
                let region = BootstrapRegion::new(file_name, &content, false);
                let wrapped = region.wrap_content(&content);
                // Replace the original content with wrapped version
                if let Some(start) = transcript.find(&content) {
                    modified = format!(
                        "{}{}{}",
                        &modified[..start],
                        wrapped,
                        &modified[start + content.len()..]
                    );
                    regions.push(region);
                }
            }
        }

        let total_char_count = regions.iter().map(|r| r.char_count).sum();
        let ctx = BootstrapContext {
            regions,
            reinjected_after_last_compact: true,
            total_char_count,
            pre_compact_hashes: Default::default(),
        };

        (modified, ctx)
    }

    /// Find bootstrap content in transcript by heuristic
    fn find_bootstrap_content<'a>(&self, transcript: &'a str, file_name: &str) -> Option<String> {
        // Try to find content between known markers first
        let start_marker = format!("{}file={}", BOOTSTRAP_REGION_START, file_name);
        if let Some(start_idx) = transcript.find(&start_marker) {
            if let Some(body_start) = transcript[start_idx..].find('>') {
                let body_start = start_idx + body_start + 1;
                if let Some(end_idx) = transcript[body_start..].find(BOOTSTRAP_REGION_END) {
                    return Some(transcript[body_start..body_start + end_idx].to_string());
                }
            }
        }

        // Heuristic: look for file name as a heading
        // AGENTS.md content typically starts with a heading like "# AGENTS" or "## AGENTS"
        if file_name == "AGENTS.md" {
            if let Some(idx) = transcript.find("# AGENTS") {
                // Find the end - next heading or end of file marker
                let end_idx = transcript[idx..]
                    .find("\n# ")
                    .or_else(|| transcript[idx..].find("\n## "))
                    .map(|i| idx + i)
                    .unwrap_or(transcript.len());
                let content = transcript[idx..end_idx].trim().to_string();
                if !content.is_empty() {
                    return Some(content);
                }
            }
        }

        // Try reading from workspace if available
        if let Some(ref workspace) = self.workspace_path {
            let file_path = workspace.join(file_name);
            if file_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    let trimmed = content.trim();
                    // Check if this content appears in the transcript
                    if transcript.contains(trimmed) {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }

        None
    }

    /// Called before compaction to store integrity hashes
    pub fn before_compact(&self, ctx: &mut BootstrapContext) {
        ctx.pre_compact_hashes.clear();
        for region in &ctx.regions {
            ctx.pre_compact_hashes
                .insert(region.region_id.clone(), region.content_hash.clone());
        }
    }

    /// Called after compaction to detect corruption.
    /// Returns list of file names that need reinject.
    pub fn after_compact(&self, transcript: &str, ctx: &mut BootstrapContext) -> Vec<String> {
        let mut to_reinject = Vec::new();

        for region in &ctx.regions {
            // Try to find the region in the modified transcript
            let start_marker = format!("{}file={}", BOOTSTRAP_REGION_START, region.file_name);
            if let Some(start_idx) = transcript.find(&start_marker) {
                if let Some(marker_end) = transcript[start_idx..].find('>') {
                    // body_content_start is right after the '>' of start marker
                    let body_content_start = start_idx + marker_end + 1;
                    if let Some(end_rel) =
                        transcript[body_content_start..].find(BOOTSTRAP_REGION_END)
                    {
                        // Extract content - strip leading newline (after >) and trailing newline (before </bootstrap>)
                        let raw_content =
                            &transcript[body_content_start..body_content_start + end_rel];
                        let content = raw_content.trim();
                        if !region.verify_integrity(content) {
                            to_reinject.push(region.file_name.clone());
                        }
                        continue;
                    }
                }
            }

            // Region not found or corrupted
            if ctx.pre_compact_hashes.contains_key(&region.region_id) {
                // Region exists in pre_compact_hashes but not found in transcript
                // This means compaction removed or corrupted it
                to_reinject.push(region.file_name.clone());
            }
        }

        if !to_reinject.is_empty() {
            ctx.reinjected_after_last_compact = false;
        }

        to_reinject
    }

    /// Generate reinject text for the specified files.
    /// Returns the full reinject block to prepend to transcript.
    pub fn reinject(
        &self,
        file_names: &[String],
        ctx: &mut BootstrapContext,
    ) -> Result<String, BootstrapProtectionError> {
        let workspace = self
            .workspace_path
            .as_ref()
            .ok_or(BootstrapProtectionError::WorkspacePathRequired)?;

        let mut reinject_blocks = Vec::new();

        for file_name in file_names {
            let file_path = workspace.join(file_name);
            let content = if file_path.exists() {
                std::fs::read_to_string(&file_path)?
            } else {
                // Try relative path
                std::fs::read_to_string(file_name)?
            };

            let region = BootstrapRegion::new(file_name, &content, true);
            reinject_blocks.push(region.wrap_content(&content));
            ctx.add_region(region);
        }

        let total = ctx.total_char_count;
        if total > self.size_limit {
            tracing::warn!(
                total_chars = total,
                limit = self.size_limit,
                "bootstrap total char count exceeds size limit"
            );
        }

        Ok(reinject_blocks.join("\n"))
    }

    /// Read bootstrap files from workspace and return as a map
    pub fn read_bootstrap_files(
        &self,
    ) -> Result<HashMap<String, String>, BootstrapProtectionError> {
        let workspace = self
            .workspace_path
            .as_ref()
            .ok_or(BootstrapProtectionError::WorkspacePathRequired)?;

        let mut files = HashMap::new();
        for file_name in &self.bootstrap_files {
            let file_path = workspace.join(file_name);
            if file_path.exists() {
                let content = std::fs::read_to_string(&file_path)?;
                files.insert(file_name.clone(), content);
            }
        }
        Ok(files)
    }

    /// Get the list of bootstrap files being protected
    pub fn bootstrap_files(&self) -> &[String] {
        &self.bootstrap_files
    }

    /// Get the workspace path if set
    pub fn workspace_path(&self) -> Option<&PathBuf> {
        self.workspace_path.as_ref()
    }
}

/// Utility: generate region marker string
pub fn make_bootstrap_marker(file_name: &str, content: &str, is_reinject: bool) -> String {
    let hash = BootstrapRegion::compute_hash(content);
    let char_count = content.chars().count();
    format!(
        "{}file={},hash={},chars={},reinject={}>",
        BOOTSTRAP_REGION_START,
        file_name,
        &hash[..12],
        char_count,
        is_reinject
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_protection() -> BootstrapProtection {
        BootstrapProtection::new()
    }

    #[test]
    fn test_bootstrap_region_new() {
        let content = "# AGENTS\n\nDo this and that.";
        let region = BootstrapRegion::new("AGENTS.md", content, false);

        assert_eq!(region.file_name, "AGENTS.md");
        assert!(!region.is_reinject);
        assert_eq!(region.char_count, content.chars().count());
        assert!(!region.region_id.is_empty());
    }

    #[test]
    fn test_bootstrap_region_hash_integrity() {
        let content = "# AGENTS\n\nDo this and that.";
        let region = BootstrapRegion::new("AGENTS.md", content, false);

        assert!(region.verify_integrity(content));
        assert!(!region.verify_integrity("# Modified content"));
        assert!(!region.verify_integrity(""));
    }

    #[test]
    fn test_bootstrap_region_wrap_content() {
        let content = "# AGENTS\n\nDo this and that.";
        let region = BootstrapRegion::new("AGENTS.md", content, false);

        let wrapped = region.wrap_content(content);
        assert!(wrapped.starts_with(BOOTSTRAP_REGION_START));
        assert!(wrapped.contains(content));
        assert!(wrapped.ends_with(BOOTSTRAP_REGION_END));
    }

    #[test]
    fn test_bootstrap_region_parse_from_marker() {
        let content = "# AGENTS\n\nDo this and that.";
        let hash = BootstrapRegion::compute_hash(content);
        let marker = format!(
            "{}file=AGENTS.md,hash={},chars={},reinject=false>",
            BOOTSTRAP_REGION_START,
            &hash[..12],
            content.chars().count()
        );

        let region = BootstrapRegion::parse_from_marker(&marker, content, Utc::now()).unwrap();
        assert_eq!(region.file_name, "AGENTS.md");
        assert!(!region.is_reinject);
    }

    #[test]
    fn test_bootstrap_context_default() {
        let ctx = BootstrapContext::default();
        assert!(ctx.regions.is_empty());
        assert!(ctx.reinjected_after_last_compact);
        assert_eq!(ctx.total_char_count, 0);
        assert!(ctx.pre_compact_hashes.is_empty());
    }

    #[test]
    fn test_bootstrap_context_add_region() {
        let mut ctx = BootstrapContext::default();
        let region = BootstrapRegion::new("AGENTS.md", "# AGENTS", false);
        let char_count = region.char_count;

        ctx.add_region(region);
        assert_eq!(ctx.regions.len(), 1);
        assert_eq!(ctx.total_char_count, char_count);
    }

    #[test]
    fn test_bootstrap_context_check_integrity() {
        let mut ctx = BootstrapContext::default();
        ctx.add_region(BootstrapRegion::new("AGENTS.md", "# AGENTS content", false));

        // Content matches - no corruption
        let corrupted = ctx.check_integrity([("AGENTS.md", "# AGENTS content")].into_iter());
        assert!(corrupted.is_empty());

        // Content modified - detected as corrupted
        let corrupted = ctx.check_integrity([("AGENTS.md", "# MODIFIED content")].into_iter());
        assert_eq!(corrupted.len(), 1);
        assert_eq!(corrupted[0], "AGENTS.md");
    }

    #[test]
    fn test_bootstrap_context_exceeds_size_limit() {
        let mut ctx = BootstrapContext::default();
        ctx.total_char_count = 70 * 1024; // 70K

        assert!(ctx.exceeds_size_limit(60 * 1024));
        assert!(!ctx.exceeds_size_limit(80 * 1024));
    }

    #[test]
    fn test_bootstrap_protection_before_compact() {
        let protection = make_test_protection();
        let mut ctx = BootstrapContext::default();
        ctx.add_region(BootstrapRegion::new("AGENTS.md", "# test", false));

        protection.before_compact(&mut ctx);

        assert_eq!(ctx.pre_compact_hashes.len(), 1);
        assert!(ctx
            .pre_compact_hashes
            .contains_key(&ctx.regions[0].region_id));
    }

    #[test]
    fn test_bootstrap_protection_after_compact_no_corruption() {
        let protection = make_test_protection();
        let mut ctx = BootstrapContext::default();
        let content = "# AGENTS\n\nTest content.";
        ctx.add_region(BootstrapRegion::new("AGENTS.md", content, false));

        // Store pre-compact hashes
        protection.before_compact(&mut ctx);

        // Transcript unchanged - no corruption
        let wrapped = ctx.regions[0].wrap_content(content);
        let to_reinject = protection.after_compact(&wrapped, &mut ctx);

        assert!(
            to_reinject.is_empty(),
            "unexpected reinject needed: {:?}",
            to_reinject
        );
        assert!(ctx.reinjected_after_last_compact);
    }

    #[test]
    fn test_bootstrap_protection_after_compact_with_corruption() {
        let protection = make_test_protection();
        let mut ctx = BootstrapContext::default();
        let original = "# AGENTS\n\nOriginal content.";
        ctx.add_region(BootstrapRegion::new("AGENTS.md", original, false));

        // Store pre-compact hashes
        protection.before_compact(&mut ctx);

        // Transcript modified by compaction
        let corrupted = "# SUMMARY: agent lost original context";
        let wrapped = ctx.regions[0].wrap_content(corrupted);
        let to_reinject = protection.after_compact(&wrapped, &mut ctx);

        assert_eq!(to_reinject.len(), 1);
        assert_eq!(to_reinject[0], "AGENTS.md");
        assert!(!ctx.reinjected_after_last_compact);
    }

    #[test]
    fn test_bootstrap_protection_reinject() {
        let temp_dir = std::env::temp_dir();
        let protection = BootstrapProtection::new()
            .with_workspace(temp_dir.clone())
            .with_bootstrap_files(vec!["AGENTS.md".to_string()]);

        // Create a test bootstrap file
        let test_file = temp_dir.join("AGENTS.md");
        std::fs::write(&test_file, "# AGENTS\n\nTest content.").unwrap();

        let mut ctx = BootstrapContext::default();
        let result = protection.reinject(&["AGENTS.md".to_string()], &mut ctx);

        assert!(result.is_ok());
        let reinject_text = result.unwrap();
        assert!(reinject_text.contains(BOOTSTRAP_REGION_START));
        assert!(reinject_text.contains("# AGENTS"));
        assert!(reinject_text.contains(BOOTSTRAP_REGION_END));
        assert!(ctx.regions[0].is_reinject);

        // Cleanup
        std::fs::remove_file(test_file).ok();
    }

    #[test]
    fn test_make_bootstrap_marker() {
        let content = "# AGENTS";
        let marker = make_bootstrap_marker("AGENTS.md", content, false);

        assert!(marker.starts_with(BOOTSTRAP_REGION_START));
        assert!(marker.contains("file=AGENTS.md"));
        assert!(marker.contains("reinject=false"));
        assert!(marker.ends_with('>'));
    }

    #[test]
    fn test_region_id_unique() {
        let r1 = BootstrapRegion::new("AGENTS.md", "# Content A", false);
        let r2 = BootstrapRegion::new("AGENTS.md", "# Content B", false);

        // Same file with different content should have different region_ids
        // (because hash is part of region_id)
        assert_ne!(r1.region_id, r2.region_id);
    }
}
