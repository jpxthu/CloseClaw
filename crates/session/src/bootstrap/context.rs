//! Bootstrap context — metadata tracking all bootstrap regions

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::types::BootstrapRegion;

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
