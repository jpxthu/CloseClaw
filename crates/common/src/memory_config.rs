//! Memory subsystem configuration types (storage/mining/dreaming/forgetting/search).

use serde::{Deserialize, Serialize};

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

// ── Dreaming subsystem ──────────────────────────────────────────────────

/// Dreaming subsystem configuration.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingConfig {
    /// Whether dreaming is enabled. `None` means inherit global default.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Dream Diary settings.
    #[serde(default)]
    pub diary: DreamingDiaryConfig,
    /// Model for lesson distillation and Dream Diary. None inherits global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Cron expression for dreaming schedule. `None` means inherit global default.
    #[serde(default)]
    pub schedule: Option<String>,
    /// Scoring dimension weights.
    #[serde(default)]
    pub scoring: DreamingScoringConfig,
    /// Score thresholds for rule promotion.
    #[serde(default)]
    pub threshold: DreamingThresholdConfig,
    /// Capacity limits.
    #[serde(default)]
    pub capacity: DreamingCapacityConfig,
}

impl DreamingConfig {
    /// Field-level merge: agent's declared fields override global.
    pub fn merge_overrides(&self, agent: &DreamingConfig) -> DreamingConfig {
        DreamingConfig {
            enabled: agent.enabled.or(self.enabled),
            diary: self.diary.merge_overrides(&agent.diary),
            model: agent.model.clone().or_else(|| self.model.clone()),
            schedule: agent.schedule.clone().or_else(|| self.schedule.clone()),
            scoring: self.scoring.merge_overrides(&agent.scoring),
            threshold: self.threshold.merge_overrides(&agent.threshold),
            capacity: self.capacity.merge_overrides(&agent.capacity),
        }
    }
}

/// Default dreaming schedule cron expression.
pub fn default_dreaming_schedule() -> String {
    "0 3 * * *".to_string()
}

/// Dream Diary configuration.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingDiaryConfig {
    /// Whether Dream Diary writing is enabled.
    /// `None` means inherit global default.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Directory path for diary files (relative to data root).
    /// `None` means inherit global default.
    #[serde(default)]
    pub path: Option<String>,
}

impl DreamingDiaryConfig {
    /// Field-level merge: agent's declared fields override global.
    pub fn merge_overrides(&self, agent: &DreamingDiaryConfig) -> DreamingDiaryConfig {
        DreamingDiaryConfig {
            enabled: agent.enabled.or(self.enabled),
            path: agent.path.clone().or_else(|| self.path.clone()),
        }
    }
}

/// Default diary path (relative to data root).
pub fn default_diary_path() -> String {
    "memory/diary/".to_string()
}

/// Scoring dimension weights for dreaming.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingScoringConfig {
    /// Entity cross-session frequency weight. `None` means inherit global default.
    #[serde(default)]
    pub frequency_weight: Option<f64>,
    /// Recency decay weight. `None` means inherit global default.
    #[serde(default)]
    pub recency_weight: Option<f64>,
    /// Owner explicitness bonus weight. `None` means inherit global default.
    #[serde(default)]
    pub explicitness_weight: Option<f64>,
    /// Cross-agent entity bonus weight. `None` means inherit global default.
    #[serde(default)]
    pub cross_agent_weight: Option<f64>,
    /// Negative signal penalty weight. `None` means inherit global default.
    #[serde(default)]
    pub negative_signal_weight: Option<f64>,
}

impl DreamingScoringConfig {
    /// Field-level merge: agent's declared fields override global.
    pub fn merge_overrides(&self, agent: &DreamingScoringConfig) -> DreamingScoringConfig {
        DreamingScoringConfig {
            frequency_weight: agent.frequency_weight.or(self.frequency_weight),
            recency_weight: agent.recency_weight.or(self.recency_weight),
            explicitness_weight: agent.explicitness_weight.or(self.explicitness_weight),
            cross_agent_weight: agent.cross_agent_weight.or(self.cross_agent_weight),
            negative_signal_weight: agent.negative_signal_weight.or(self.negative_signal_weight),
        }
    }
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

/// Dreaming threshold configuration.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingThresholdConfig {
    /// Absolute score threshold for rule promotion. `None` means inherit global default.
    #[serde(default)]
    pub absolute: Option<f64>,
    /// Relative score threshold ratio. `None` means inherit global default.
    #[serde(default)]
    pub relative: Option<f64>,
}

impl DreamingThresholdConfig {
    /// Field-level merge: agent's declared fields override global.
    pub fn merge_overrides(&self, agent: &DreamingThresholdConfig) -> DreamingThresholdConfig {
        DreamingThresholdConfig {
            absolute: agent.absolute.or(self.absolute),
            relative: agent.relative.or(self.relative),
        }
    }
}

/// Default threshold absolute value.
pub fn default_threshold_absolute() -> f64 {
    2.0
}

/// Default threshold relative value.
pub fn default_threshold_relative() -> f64 {
    0.3
}

/// Dreaming capacity configuration.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingCapacityConfig {
    /// Maximum number of rules in MEMORY.md. `None` means inherit global default.
    #[serde(default)]
    pub max_rules: Option<usize>,
}

impl DreamingCapacityConfig {
    /// Field-level merge: agent's declared fields override global.
    pub fn merge_overrides(&self, agent: &DreamingCapacityConfig) -> DreamingCapacityConfig {
        DreamingCapacityConfig {
            max_rules: agent.max_rules.or(self.max_rules),
        }
    }
}

/// Default capacity max rules.
pub fn default_capacity_max_rules() -> usize {
    20
}

// ── Forgetting subsystem ──────────────────────────────────────────────

/// Forgetting subsystem configuration.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgettingConfig {
    /// Initial TTL in days for new events. `None` means inherit global default.
    #[serde(default)]
    pub initial_ttl_days: Option<i64>,
    /// Days to extend `expires_at` when Miner 1 dedup re-identifies an entity.
    /// `None` means inherit global default.
    #[serde(default)]
    pub reidentify_extension_days: Option<i64>,
    /// Days to extend `expires_at` on active-searcher injection hit.
    /// `None` means inherit global default.
    #[serde(default)]
    pub injection_extension_days: Option<i64>,
}

impl ForgettingConfig {
    /// Field-level merge: agent's declared fields override global.
    pub fn merge_overrides(&self, agent: &ForgettingConfig) -> ForgettingConfig {
        ForgettingConfig {
            initial_ttl_days: agent.initial_ttl_days.or(self.initial_ttl_days),
            reidentify_extension_days: agent
                .reidentify_extension_days
                .or(self.reidentify_extension_days),
            injection_extension_days: agent
                .injection_extension_days
                .or(self.injection_extension_days),
        }
    }
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

// ── Storage paths ───────────────────────────────────────────────────────

/// Storage paths for memory subsystem.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStorageConfig {
    /// SQLite database file path (relative to data root).
    /// `None` means inherit global default.
    #[serde(default)]
    pub db_path: Option<String>,
    /// MEMORY.md file path (relative to data root).
    /// `None` means inherit global default.
    #[serde(default)]
    pub memory_md_path: Option<String>,
}

impl MemoryStorageConfig {
    /// Field-level merge: agent's declared fields override global,
    /// except `memory_md_path` which is always global (shared across all agents,
    /// per design doc: "MEMORY.md is a globally shared single rules file").
    pub fn merge_overrides(&self, agent: &MemoryStorageConfig) -> MemoryStorageConfig {
        MemoryStorageConfig {
            db_path: agent.db_path.clone().or_else(|| self.db_path.clone()),
            memory_md_path: self.memory_md_path.clone(),
        }
    }
}

/// Default database file path.
pub fn default_db_path() -> String {
    "memory/memory.db".to_string()
}

/// Default MEMORY.md file path.
pub fn default_memory_md_path() -> String {
    "memory/MEMORY.md".to_string()
}

// ── Mining subsystem ────────────────────────────────────────────────────

/// Mining subsystem configuration.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiningConfig {
    /// Whether mining is enabled. `None` means inherit global default.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Model for Miner 1 and Miner 2. None inherits global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Maximum events per mining session. `None` means inherit global default.
    #[serde(default)]
    pub max_events_per_session: Option<i32>,
    /// Dedup window in days for Miner 1. `None` means inherit global default.
    #[serde(default)]
    pub dedup_window_days: Option<i32>,
    /// Transcript clean rules.
    #[serde(default)]
    pub transcript_clean_rules: TranscriptCleanRules,
}

impl MiningConfig {
    /// Field-level merge: agent's declared fields override global.
    pub fn merge_overrides(&self, agent: &MiningConfig) -> MiningConfig {
        MiningConfig {
            enabled: agent.enabled.or(self.enabled),
            model: agent.model.clone().or_else(|| self.model.clone()),
            max_events_per_session: agent.max_events_per_session.or(self.max_events_per_session),
            dedup_window_days: agent.dedup_window_days.or(self.dedup_window_days),
            transcript_clean_rules: self
                .transcript_clean_rules
                .merge_overrides(&agent.transcript_clean_rules),
        }
    }
}

/// Default max events per session.
pub fn default_mining_max_events_per_session() -> i32 {
    10
}

/// Default dedup window in days.
pub fn default_mining_dedup_window_days() -> i32 {
    30
}

// ── Transcript clean rules ─────────────────────────────────────────────

/// Transcript clean rules for mining.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptCleanRules {
    /// Minimum conversation turns required. `None` means inherit global default.
    #[serde(default)]
    pub min_turns: Option<i32>,
    /// Minimum owner messages required. `None` means inherit global default.
    #[serde(default)]
    pub min_owner_msgs: Option<i32>,
    /// Transcript output format. `None` means inherit global default.
    #[serde(default)]
    pub format: Option<String>,
}

impl TranscriptCleanRules {
    /// Field-level merge: agent's declared fields override global.
    pub fn merge_overrides(&self, agent: &TranscriptCleanRules) -> TranscriptCleanRules {
        TranscriptCleanRules {
            min_turns: agent.min_turns.or(self.min_turns),
            min_owner_msgs: agent.min_owner_msgs.or(self.min_owner_msgs),
            format: agent.format.clone().or_else(|| self.format.clone()),
        }
    }
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

// ── Search subsystem ────────────────────────────────────────────────────

/// Active search subsystem configuration.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchConfig {
    /// Whether active search is enabled. `None` means inherit global default.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Model for concept extraction. None inherits global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Number of recent conversation turns for query concept extraction.
    /// `None` means inherit global default.
    #[serde(default)]
    pub context_turns: Option<usize>,
    /// Search timeout in milliseconds. `None` means inherit global default.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Maximum summary character count. `None` means inherit global default.
    #[serde(default)]
    pub max_summary_chars: Option<usize>,
    /// Minimum entity hit count. `None` means inherit global default.
    #[serde(default)]
    pub min_entity_hits: Option<u32>,
    /// Maximum event summaries to inject. `None` means inherit global default.
    #[serde(default)]
    pub top_k_events: Option<usize>,
}

impl SearchConfig {
    /// Field-level merge: agent's declared fields override global.
    pub fn merge_overrides(&self, agent: &SearchConfig) -> SearchConfig {
        SearchConfig {
            enabled: agent.enabled.or(self.enabled),
            model: agent.model.clone().or_else(|| self.model.clone()),
            context_turns: agent.context_turns.or(self.context_turns),
            timeout_ms: agent.timeout_ms.or(self.timeout_ms),
            max_summary_chars: agent.max_summary_chars.or(self.max_summary_chars),
            min_entity_hits: agent.min_entity_hits.or(self.min_entity_hits),
            top_k_events: agent.top_k_events.or(self.top_k_events),
        }
    }
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
