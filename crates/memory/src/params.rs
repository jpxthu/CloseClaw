//! Memory-owned parameter types.
//!
//! The memory crate has no dependency on the config crate; composition
//! layers (daemon / gateway) map resolved config values onto these types
//! at assembly time via field-level copies.
//!
//! Option-field semantics mirror the config side: `None` means "not
//! declared", and the `default_*` functions below provide the fallback
//! values (behaviour identical to the previous config-side defaults).

// ── Transcript clean rules ──────────────────────────────────────────────

/// Transcript clean rules for mining.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TranscriptCleanRules {
    /// Minimum conversation turns required.
    pub min_turns: Option<i32>,
    /// Minimum owner messages required.
    pub min_owner_msgs: Option<i32>,
    /// Transcript output format.
    pub format: Option<String>,
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

// ── Miner params (mining.* + miner-consumed forgetting.* fields) ────────

/// Parameters consumed by the memory miner.
///
/// Aggregates `mining.*` fields plus the `forgetting.*` fields the miner
/// reads (`initial_ttl_days`, `reidentify_extension_days`).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MinerParams {
    /// Whether mining is enabled.
    pub enabled: Option<bool>,
    /// Model for Miner 1 and Miner 2.
    pub model: Option<String>,
    /// Maximum events per mining session.
    pub max_events_per_session: Option<i32>,
    /// Dedup window in days for Miner 1.
    pub dedup_window_days: Option<i32>,
    /// Transcript clean rules.
    pub clean_rules: TranscriptCleanRules,
    /// Initial TTL in days for new events.
    pub initial_ttl_days: Option<i64>,
    /// Days to extend `expires_at` when Miner 1 dedup re-identifies an entity.
    pub reidentify_extension_days: Option<i64>,
}

/// Default max events per session.
pub fn default_mining_max_events_per_session() -> i32 {
    10
}

/// Default dedup window in days.
pub fn default_mining_dedup_window_days() -> i32 {
    30
}

/// Default initial TTL in days for new events.
pub fn default_forgetting_initial_ttl_days() -> i64 {
    90
}

/// Default reidentify extension days for Miner 1 dedup hits.
pub fn default_forgetting_reidentify_extension_days() -> i64 {
    90
}

// ── Dreaming params (incl. scoring) ─────────────────────────────────────

/// Parameters consumed by the dreaming pipeline.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DreamingParams {
    /// Whether dreaming is enabled.
    pub enabled: Option<bool>,
    /// Dream Diary settings.
    pub diary: DreamingDiaryParams,
    /// Model for lesson distillation and Dream Diary.
    pub model: Option<String>,
    /// Cron expression for dreaming schedule.
    pub schedule: Option<String>,
    /// Scoring dimension weights.
    pub scoring: DreamingScoringParams,
    /// Score thresholds for rule promotion.
    pub threshold: DreamingThresholdParams,
    /// Capacity limits.
    pub capacity: DreamingCapacityParams,
}

/// Dream Diary parameters.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DreamingDiaryParams {
    /// Whether Dream Diary writing is enabled.
    pub enabled: Option<bool>,
    /// Directory path for diary files (relative to data root).
    pub path: Option<String>,
}

/// Default diary path (relative to data root).
pub fn default_diary_path() -> String {
    "memory/diary/".to_string()
}

/// Default MEMORY.md file path.
pub fn default_memory_md_path() -> String {
    "memory/MEMORY.md".to_string()
}

/// Scoring dimension weights for dreaming.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DreamingScoringParams {
    /// Entity cross-session frequency weight.
    pub frequency_weight: Option<f64>,
    /// Recency decay weight.
    pub recency_weight: Option<f64>,
    /// Owner explicitness bonus weight.
    pub explicitness_weight: Option<f64>,
    /// Cross-agent entity bonus weight.
    pub cross_agent_weight: Option<f64>,
    /// Negative signal penalty weight.
    pub negative_signal_weight: Option<f64>,
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

/// Dreaming threshold parameters.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DreamingThresholdParams {
    /// Absolute score threshold for rule promotion.
    pub absolute: Option<f64>,
    /// Relative score threshold ratio.
    pub relative: Option<f64>,
}

/// Default threshold absolute value.
pub fn default_threshold_absolute() -> f64 {
    2.0
}

/// Default threshold relative value.
pub fn default_threshold_relative() -> f64 {
    0.3
}

/// Dreaming capacity parameters.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DreamingCapacityParams {
    /// Maximum number of rules in MEMORY.md.
    pub max_rules: Option<usize>,
}

/// Default capacity max rules.
pub fn default_capacity_max_rules() -> usize {
    20
}

// ── Search params (active-searcher consumed fields) ─────────────────────

/// Parameters consumed by the active searcher.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SearchParams {
    /// Whether active search is enabled.
    pub enabled: Option<bool>,
    /// Model for concept extraction.
    pub model: Option<String>,
    /// Number of recent conversation turns for query concept extraction.
    pub context_turns: Option<usize>,
    /// Search timeout in milliseconds.
    pub timeout_ms: Option<u64>,
    /// Maximum summary character count.
    pub max_summary_chars: Option<usize>,
    /// Minimum entity hit count.
    pub min_entity_hits: Option<u32>,
    /// Maximum event summaries to inject.
    pub top_k_events: Option<usize>,
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

// ── Forgetting params (active-searcher consumed fields) ─────────────────

/// Forgetting parameters consumed by the active searcher.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ForgettingParams {
    /// Days to extend `expires_at` on active-searcher injection hit.
    pub injection_extension_days: Option<i64>,
}

/// Default injection extension days for active-searcher hits.
pub fn default_forgetting_injection_extension_days() -> i64 {
    7
}
