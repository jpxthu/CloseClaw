//! Bootstrap protection service

use std::collections::HashMap;
use std::path::PathBuf;

use super::context::BootstrapContext;
use super::types::{BootstrapProtectionError, BootstrapRegion, BOOTSTRAP_REGION_END};

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
            bootstrap_files: super::bootstrap_file_list(super::BootstrapMode::Full)
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
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

    /// Set bootstrap files based on a [`BootstrapMode`].
    ///
    /// Minimal mode: `["AGENTS.md", "SOUL.md", "IDENTITY.md", "USER.md", "TOOLS.md"]`
    /// Full mode: `["AGENTS.md", "SOUL.md", "IDENTITY.md", "USER.md", "TOOLS.md", "BOOTSTRAP.md", "MEMORY.md"]`
    pub fn with_mode(mut self, mode: super::BootstrapMode) -> Self {
        let names = super::bootstrap_file_list(mode);
        self.bootstrap_files = names.into_iter().map(|s| s.to_string()).collect();
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
        use super::types::BOOTSTRAP_REGION_START;

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
        use super::types::BOOTSTRAP_REGION_START;

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
