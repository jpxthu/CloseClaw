//! Memory subsystem configuration types (storage/mining/dreaming/forgetting/search).
//!
//! Only [`MemoryConfig`] is part of the crate's publicly reachable surface
//! (it is the entry listed in `docs/design/common/shared-types.md`); the
//! sub-configuration structs live in the private-ish [`sub_configs`] module
//! and are not re-exported at any level. `default_*` helpers stay here for
//! downstream consumers (they are not part of the admission comparison).

pub(crate) mod sub_configs;

use serde::{Deserialize, Serialize};

use sub_configs::{
    DreamingConfig, ForgettingConfig, MemoryStorageConfig, MiningConfig, SearchConfig,
};

// ── Memory subsystem configuration ──────────────────────────────────────

/// Memory subsystem configuration.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConfig {
    /// Storage paths for memory subsystem files.
    #[serde(default)]
    pub storage: MemoryStorageConfig,
    /// Mining subsystem configuration.
    #[serde(default)]
    pub mining: MiningConfig,
    /// Dreaming subsystem configuration.
    #[serde(default)]
    pub dreaming: DreamingConfig,
    /// Active search subsystem configuration.
    #[serde(default)]
    pub search: SearchConfig,
    /// Forgetting subsystem configuration.
    #[serde(default)]
    pub forgetting: ForgettingConfig,
}

impl MemoryConfig {
    /// Field-level merge: agent's declared fields override global,
    /// undeclared fields inherit global values.
    pub fn merge_overrides(&self, agent: &MemoryConfig) -> MemoryConfig {
        MemoryConfig {
            storage: self.storage.merge_overrides(&agent.storage),
            mining: self.mining.merge_overrides(&agent.mining),
            dreaming: self.dreaming.merge_overrides(&agent.dreaming),
            search: self.search.merge_overrides(&agent.search),
            forgetting: self.forgetting.merge_overrides(&agent.forgetting),
        }
    }
}

// ── Default values ──────────────────────────────────────────────────────

/// Default dreaming schedule cron expression.
pub fn default_dreaming_schedule() -> String {
    "0 3 * * *".to_string()
}

/// Default diary path (relative to data root).
pub fn default_diary_path() -> String {
    "memory/diary/".to_string()
}

/// Default scoring frequency weight.
pub fn default_scoring_frequency() -> f64 {
    1.0
}

/// Default scoring recency weight.
pub fn default_scoring_recency() -> f64 {
    0.5
}

/// Default scoring explicitness weight.
pub fn default_scoring_explicitness() -> f64 {
    1.5
}

/// Default scoring cross-agent weight.
pub fn default_scoring_cross_agent() -> f64 {
    1.3
}

/// Default scoring negative signal weight.
pub fn default_scoring_negative_signal() -> f64 {
    -0.5
}

/// Default threshold absolute value.
pub fn default_threshold_absolute() -> f64 {
    2.0
}

/// Default threshold relative value.
pub fn default_threshold_relative() -> f64 {
    0.3
}

/// Default capacity max rules.
pub fn default_capacity_max_rules() -> usize {
    20
}

/// Default initial TTL in days for new events.
pub fn default_forgetting_initial_ttl_days() -> i64 {
    90
}

/// Default reidentify extension days for Miner 1 dedup hits.
pub fn default_forgetting_reidentify_extension_days() -> i64 {
    90
}

/// Default injection extension days for active-searcher hits.
pub fn default_forgetting_injection_extension_days() -> i64 {
    7
}

/// Default database file path.
pub fn default_db_path() -> String {
    "memory/memory.db".to_string()
}

/// Default MEMORY.md file path.
pub fn default_memory_md_path() -> String {
    "memory/MEMORY.md".to_string()
}

/// Default max events per session.
pub fn default_mining_max_events_per_session() -> i32 {
    10
}

/// Default dedup window in days.
pub fn default_mining_dedup_window_days() -> i32 {
    30
}

/// Default minimum turns.
pub fn default_transcript_min_turns() -> i32 {
    5
}

/// Default minimum owner messages.
pub fn default_transcript_min_owner_msgs() -> i32 {
    5
}

/// Default transcript format.
pub fn default_transcript_format() -> String {
    "md".to_string()
}

/// Default context turns for search.
pub fn default_search_context_turns() -> usize {
    5
}

/// Default search timeout in milliseconds.
pub fn default_search_timeout_ms() -> u64 {
    3000
}

/// Default max summary characters.
pub fn default_search_max_summary_chars() -> usize {
    500
}

/// Default minimum entity hits.
pub fn default_search_min_entity_hits() -> u32 {
    1
}

/// Default top K events.
pub fn default_search_top_k_events() -> usize {
    3
}
