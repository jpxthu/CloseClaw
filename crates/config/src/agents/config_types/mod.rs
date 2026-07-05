//! Agent configuration types — config.json and permissions.json structures
//! for per-agent config files.
//!
//! Migrated from `closeclaw-common::agent_config`.
//! Design: `docs/agent/MULTI_AGENT_ARCHITECTURE.md`

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use closeclaw_common::BootstrapMode;

/// Agent's own configuration (stored as config.json in the agent's directory).
///
/// Permissions are stored in a separate `permissions.json` file, not inline
/// in `config.json` (per design doc).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    /// Unique identifier for this agent.
    pub id: String,
    /// Human-readable name.
    #[serde(default)]
    pub name: Option<String>,
    /// Parent agent ID (if this agent was spawned by another).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Default LLM model for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSpec>,
    /// Working directory path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Directory for bootstrap files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_dir: Option<String>,
    /// Bootstrap file loading mode.
    #[serde(default)]
    pub bootstrap_mode: Option<BootstrapMode>,
    /// Available skill names; `["*"]` means all skills are available.
    #[serde(default = "default_all")]
    pub skills: Vec<String>,
    /// Available tool names whitelist.
    #[serde(default = "default_all")]
    pub tools: Vec<String>,
    /// Disallowed tool names blacklist.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// Sub-agent spawn control parameters.
    #[serde(default)]
    pub subagents: SubagentsConfig,
    /// Memory subsystem configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryConfig>,
}

fn default_all() -> Vec<String> {
    vec!["*".to_string()]
}

/// Sub-agent spawn control configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentsConfig {
    /// Whitelist of allowed target agent IDs; `["*"]` means no restriction.
    #[serde(default = "default_all")]
    pub allow_agents: Vec<String>,
    /// Whether agentId must be explicitly specified when spawning.
    #[serde(default)]
    pub require_agent_id: Option<bool>,
    /// Maximum nested spawn depth.
    #[serde(default)]
    pub max_spawn_depth: Option<u32>,
    /// Maximum concurrent active child sessions.
    #[serde(default)]
    pub max_children: Option<u32>,
    /// Default child agent ID (used when spawn omits agentId).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_child_agent: Option<String>,
    /// Model override for child agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSpec>,
}

impl Default for SubagentsConfig {
    fn default() -> Self {
        Self {
            allow_agents: default_all(),
            require_agent_id: None,
            max_spawn_depth: None,
            max_children: None,
            default_child_agent: None,
            model: None,
        }
    }
}

/// Agent model specification with optional fallback list.
///
/// Supports two JSON formats for backward compatibility:
/// - String: `"gpt-4o"` → single model, no fallback
/// - Object: `{"primary": "gpt-4o", "fallback": ["claude-3"]}` → with fallback list
///
/// The primary model is always the first to try. Fallback models are tried
/// in order if the primary is unavailable; actual fallback logic lives in
/// the LLM layer (`unified_fallback.rs`), not here.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelSpec {
    pub primary: String,
    pub fallback: Vec<String>,
}

impl ModelSpec {
    /// Create a ModelSpec with a single primary model and no fallbacks.
    pub fn single(model: impl Into<String>) -> Self {
        Self {
            primary: model.into(),
            fallback: Vec::new(),
        }
    }

    /// Create a ModelSpec with a primary model and a list of fallbacks.
    pub fn with_fallback(primary: impl Into<String>, fallback: Vec<String>) -> Self {
        Self {
            primary: primary.into(),
            fallback,
        }
    }
}

impl fmt::Display for ModelSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.primary)
    }
}

impl Serialize for ModelSpec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.fallback.is_empty() {
            serializer.serialize_str(&self.primary)
        } else {
            let mut state = serializer.serialize_struct("ModelSpec", 2)?;
            state.serialize_field("primary", &self.primary)?;
            state.serialize_field("fallback", &self.fallback)?;
            state.end()
        }
    }
}

struct ModelSpecVisitor;

impl<'de> Visitor<'de> for ModelSpecVisitor {
    type Value = ModelSpec;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a model name string or {primary, fallback} object")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<ModelSpec, E> {
        Ok(ModelSpec::single(value))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<ModelSpec, E> {
        Ok(ModelSpec::single(value))
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<ModelSpec, M::Error> {
        let mut primary = None;
        let mut fallback = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "primary" => {
                    if primary.is_some() {
                        return Err(de::Error::duplicate_field("primary"));
                    }
                    primary = Some(map.next_value()?);
                }
                "fallback" => {
                    if fallback.is_some() {
                        return Err(de::Error::duplicate_field("fallback"));
                    }
                    fallback = Some(map.next_value()?);
                }
                _ => {
                    let _ = map.next_value::<de::IgnoredAny>()?;
                }
            }
        }

        let primary = primary.ok_or_else(|| de::Error::missing_field("primary"))?;
        let fallback = fallback.unwrap_or_default();

        Ok(ModelSpec { primary, fallback })
    }
}

impl<'de> Deserialize<'de> for ModelSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ModelSpecVisitor)
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: None,
            parent_id: None,
            model: None,
            workspace: None,
            agent_dir: None,
            bootstrap_mode: None,
            skills: default_all(),
            tools: default_all(),
            disallowed_tools: Vec::new(),
            subagents: SubagentsConfig::default(),
            memory: None,
        }
    }
}

/// Memory subsystem configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_searcher: Option<ActiveSearcherOverride>,
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
}

/// Dreaming subsystem configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingConfig {
    /// Whether dreaming is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Dream Diary settings.
    #[serde(default)]
    pub diary: DreamingDiaryConfig,
    /// Model for lesson distillation and Dream Diary. None inherits global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Cron expression for dreaming schedule.
    #[serde(default = "default_dreaming_schedule")]
    pub schedule: String,
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

impl Default for DreamingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            diary: DreamingDiaryConfig::default(),
            model: None,
            schedule: default_dreaming_schedule(),
            scoring: DreamingScoringConfig::default(),
            threshold: DreamingThresholdConfig::default(),
            capacity: DreamingCapacityConfig::default(),
        }
    }
}

fn default_dreaming_schedule() -> String {
    "0 3 * * *".to_string()
}

/// Dream Diary configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingDiaryConfig {
    /// Whether Dream Diary writing is enabled.
    /// Defaults to `true` when dreaming is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Directory path for diary files (relative to data root).
    #[serde(default = "default_diary_path")]
    pub path: String,
}

#[allow(clippy::derivable_impls)]
impl Default for DreamingDiaryConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            path: default_diary_path(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_diary_path() -> String {
    "memory/diary/".to_string()
}

/// Scoring dimension weights for dreaming.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingScoringConfig {
    /// Entity cross-session frequency weight.
    #[serde(default = "default_scoring_frequency")]
    pub frequency_weight: f64,
    /// Recency decay weight.
    #[serde(default = "default_scoring_recency")]
    pub recency_weight: f64,
    /// Owner explicitness bonus weight.
    #[serde(default = "default_scoring_explicitness")]
    pub explicitness_weight: f64,
    /// Cross-agent entity bonus weight.
    #[serde(default = "default_scoring_cross_agent")]
    pub cross_agent_weight: f64,
    /// Negative signal penalty weight.
    #[serde(default = "default_scoring_negative_signal")]
    pub negative_signal_weight: f64,
}

impl Default for DreamingScoringConfig {
    fn default() -> Self {
        Self {
            frequency_weight: default_scoring_frequency(),
            recency_weight: default_scoring_recency(),
            explicitness_weight: default_scoring_explicitness(),
            cross_agent_weight: default_scoring_cross_agent(),
            negative_signal_weight: default_scoring_negative_signal(),
        }
    }
}

fn default_scoring_frequency() -> f64 {
    1.0
}

fn default_scoring_recency() -> f64 {
    0.5
}

fn default_scoring_explicitness() -> f64 {
    1.5
}

fn default_scoring_cross_agent() -> f64 {
    1.3
}

fn default_scoring_negative_signal() -> f64 {
    -0.5
}

/// Dreaming threshold configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingThresholdConfig {
    /// Absolute score threshold for rule promotion.
    #[serde(default = "default_threshold_absolute")]
    pub absolute: f64,
    /// Relative score threshold ratio.
    #[serde(default = "default_threshold_relative")]
    pub relative: f64,
}

impl Default for DreamingThresholdConfig {
    fn default() -> Self {
        Self {
            absolute: default_threshold_absolute(),
            relative: default_threshold_relative(),
        }
    }
}

fn default_threshold_absolute() -> f64 {
    2.0
}

fn default_threshold_relative() -> f64 {
    0.3
}

/// Dreaming capacity configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingCapacityConfig {
    /// Maximum number of rules in MEMORY.md.
    #[serde(default = "default_capacity_max_rules")]
    pub max_rules: usize,
}

impl Default for DreamingCapacityConfig {
    fn default() -> Self {
        Self {
            max_rules: default_capacity_max_rules(),
        }
    }
}

fn default_capacity_max_rules() -> usize {
    20
}

/// Storage paths for memory subsystem.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStorageConfig {
    /// SQLite database file path (relative to data root).
    #[serde(default = "default_db_path")]
    pub db_path: String,
    /// MEMORY.md file path (relative to data root).
    /// Used by system prompt provider and dreaming output.
    #[serde(default = "default_memory_md_path")]
    pub memory_md_path: String,
}

#[allow(clippy::derivable_impls)]
impl Default for MemoryStorageConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            memory_md_path: default_memory_md_path(),
        }
    }
}

fn default_db_path() -> String {
    "memory/memory.db".to_string()
}

fn default_memory_md_path() -> String {
    "memory/MEMORY.md".to_string()
}

/// Transcript clean rules for mining.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptCleanRules {
    /// Minimum conversation turns required.
    #[serde(default = "default_transcript_min_turns")]
    pub min_turns: i32,
    /// Minimum owner messages required.
    #[serde(default = "default_transcript_min_owner_msgs")]
    pub min_owner_msgs: i32,
    /// Transcript output format.
    #[serde(default = "default_transcript_format")]
    pub format: String,
}

impl Default for TranscriptCleanRules {
    fn default() -> Self {
        Self {
            min_turns: default_transcript_min_turns(),
            min_owner_msgs: default_transcript_min_owner_msgs(),
            format: default_transcript_format(),
        }
    }
}

fn default_transcript_min_turns() -> i32 {
    5
}

fn default_transcript_min_owner_msgs() -> i32 {
    5
}

fn default_transcript_format() -> String {
    "md".to_string()
}

/// Mining subsystem configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiningConfig {
    /// Whether mining is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Model for Miner 1 and Miner 2. None inherits global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Maximum events per mining session.
    #[serde(default = "default_mining_max_events_per_session")]
    pub max_events_per_session: i32,
    /// Dedup window in days for Miner 1.
    #[serde(default = "default_mining_dedup_window_days")]
    pub dedup_window_days: i32,
    /// Transcript clean rules.
    #[serde(default)]
    pub transcript_clean_rules: TranscriptCleanRules,
}

impl Default for MiningConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: None,
            max_events_per_session: default_mining_max_events_per_session(),
            dedup_window_days: default_mining_dedup_window_days(),
            transcript_clean_rules: TranscriptCleanRules::default(),
        }
    }
}

fn default_mining_max_events_per_session() -> i32 {
    10
}

fn default_mining_dedup_window_days() -> i32 {
    30
}

/// Active search subsystem configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchConfig {
    /// Whether active search is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Model for concept extraction. None inherits global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Number of recent conversation turns for query concept extraction.
    #[serde(default = "default_search_context_turns")]
    pub context_turns: usize,
    /// Search timeout in milliseconds.
    #[serde(default = "default_search_timeout_ms")]
    pub timeout_ms: u64,
    /// Maximum summary character count.
    #[serde(default = "default_search_max_summary_chars")]
    pub max_summary_chars: usize,
    /// Minimum entity hit count.
    #[serde(default = "default_search_min_entity_hits")]
    pub min_entity_hits: u32,
    /// Maximum event summaries to inject.
    #[serde(default = "default_search_top_k_events")]
    pub top_k_events: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: None,
            context_turns: default_search_context_turns(),
            timeout_ms: default_search_timeout_ms(),
            max_summary_chars: default_search_max_summary_chars(),
            min_entity_hits: default_search_min_entity_hits(),
            top_k_events: default_search_top_k_events(),
        }
    }
}

fn default_search_context_turns() -> usize {
    5
}

fn default_search_timeout_ms() -> u64 {
    3000
}

fn default_search_max_summary_chars() -> usize {
    500
}

fn default_search_min_entity_hits() -> u32 {
    1
}

fn default_search_top_k_events() -> usize {
    3
}

/// Active-searcher overrides — all fields optional.
/// Missing fields fall back to defaults (model from agent global).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSearcherOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_summary_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_entity_hits: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k_events: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_turns: Option<usize>,
}

/// Permission limits for a single action category.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PermissionLimits {
    /// Allowed commands (for exec).
    #[serde(default)]
    pub commands: Vec<String>,
    /// Allowed paths (for file_read/file_write).
    #[serde(default)]
    pub paths: Vec<String>,
    /// Timeout limit in milliseconds (for exec).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Permissions for a single action category.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ActionPermission {
    /// Whether this action is allowed.
    #[serde(default)]
    pub allowed: bool,
    /// Optional limits when allowed.
    #[serde(default)]
    pub limits: PermissionLimits,
}

/// Full permissions configuration for an agent (stored as permissions.json).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AgentPermissions {
    /// Agent identifier these permissions apply to.
    pub agent_id: String,
    /// Permission rules by action category.
    #[serde(default)]
    pub permissions: HashMap<String, ActionPermission>,
    /// ID of the agent from which these permissions are inherited.
    #[serde(default)]
    pub inherited_from: Option<String>,
}

impl AgentPermissions {
    /// Check if a specific action is permitted.
    pub fn is_allowed(&self, action: &str) -> bool {
        self.permissions
            .get(action)
            .map(|p| p.allowed)
            .unwrap_or(false)
    }

    /// Compute the intersection of this agent's permissions with a parent's.
    ///
    /// Seven dimensions: exec, file_read, file_write, network, spawn,
    /// tool_call, config_write.
    ///
    /// - Both Allow → Allow
    /// - Either Deny or absent → Deny
    /// - Result `agent_id` = self.agent_id, `inherited_from` = Some(parent.agent_id)
    /// - Limits: commands/paths → set intersection; timeout_ms → min;
    ///   Deny dimensions get default limits.
    /// - None means no restriction: both None → None, one None → other's Some,
    ///   both Some → min.
    pub fn intersect(&self, parent: &AgentPermissions) -> Self {
        let dimensions = [
            "exec",
            "file_read",
            "file_write",
            "network",
            "spawn",
            "tool_call",
            "config_write",
        ];

        let mut permissions = HashMap::with_capacity(dimensions.len());

        for &dim in &dimensions {
            let self_perm = self.permissions.get(dim);
            let parent_perm = parent.permissions.get(dim);

            let self_allowed = self_perm.map(|p| p.allowed).unwrap_or(false);
            let parent_allowed = parent_perm.map(|p| p.allowed).unwrap_or(false);

            if self_allowed && parent_allowed {
                let self_limits = self_perm.map(|p| &p.limits);
                let parent_limits = parent_perm.map(|p| &p.limits);
                let limits = PermissionLimits {
                    commands: intersect_vec(
                        self_limits.map(|l| &l.commands),
                        parent_limits.map(|l| &l.commands),
                    ),
                    paths: intersect_vec(
                        self_limits.map(|l| &l.paths),
                        parent_limits.map(|l| &l.paths),
                    ),
                    timeout_ms: intersect_option_min(
                        self_limits.and_then(|l| l.timeout_ms),
                        parent_limits.and_then(|l| l.timeout_ms),
                    ),
                };
                permissions.insert(
                    dim.to_string(),
                    ActionPermission {
                        allowed: true,
                        limits,
                    },
                );
            } else {
                permissions.insert(
                    dim.to_string(),
                    ActionPermission {
                        allowed: false,
                        limits: PermissionLimits::default(),
                    },
                );
            }
        }

        Self {
            agent_id: self.agent_id.clone(),
            permissions,
            inherited_from: Some(parent.agent_id.clone()),
        }
    }

    /// Returns true if all seven permission dimensions are denied or absent.
    pub fn is_fully_denied(&self) -> bool {
        ![
            "exec",
            "file_read",
            "file_write",
            "network",
            "spawn",
            "tool_call",
            "config_write",
        ]
        .iter()
        .any(|&dim| self.permissions.get(dim).is_some_and(|p| p.allowed))
    }
}

/// Set intersection: if both have some, return common elements;
/// if either is None (no restriction), take the other's value;
/// if both None → None.
pub(crate) fn intersect_vec<T: Eq + std::hash::Hash + Clone>(
    a: Option<&Vec<T>>,
    b: Option<&Vec<T>>,
) -> Vec<T> {
    match (a, b) {
        (Some(a), Some(b)) => a.iter().filter(|item| b.contains(item)).cloned().collect(),
        (Some(a), None) | (None, Some(a)) => a.clone(),
        (None, None) => Vec::new(),
    }
}

/// Minimum of two optional values; if either is None (no restriction),
/// the result is the other's value.
pub(crate) fn intersect_option_min(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests;
