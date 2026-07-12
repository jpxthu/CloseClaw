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
    /// Sub-agent maximum execution duration (seconds).
    /// Falls back to global config when unspecified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Default child agent ID (used when spawn omits agentId).
    ///
    /// **Deprecated**: This field is no longer used in agentId resolution.
    /// When spawn omits agentId, the parent agent's own ID is always used
    /// (design doc §Spawn 控制流程 ④). Kept for config file compatibility;
    /// ignored by SpawnController.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[deprecated(note = "Ignored: spawn now always uses parent agent ID as default")]
    pub default_child_agent: Option<String>,
    /// Model override for child agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSpec>,
}

impl Default for SubagentsConfig {
    #[allow(deprecated)] // default_child_agent is deprecated; included for config backward compat
    fn default() -> Self {
        Self {
            allow_agents: default_all(),
            require_agent_id: None,
            max_spawn_depth: None,
            max_children: None,
            timeout: None,
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
    /// Entity type weight dimension scoring weight. `None` means inherit global default.
    #[serde(default)]
    pub entity_type_weight_weight: Option<f64>,
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
            entity_type_weight_weight: agent
                .entity_type_weight_weight
                .or(self.entity_type_weight_weight),
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

/// Default scoring entity type weight.
pub fn default_scoring_entity_type_weight() -> f64 {
    1.0
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
    /// Field-level merge: agent's declared fields override global.
    pub fn merge_overrides(&self, agent: &MemoryStorageConfig) -> MemoryStorageConfig {
        MemoryStorageConfig {
            db_path: agent.db_path.clone().or_else(|| self.db_path.clone()),
            memory_md_path: agent
                .memory_md_path
                .clone()
                .or_else(|| self.memory_md_path.clone()),
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

// ── Permission types (unchanged) ────────────────────────────────────────

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
    /// Eight dimensions: command, file_read, file_write, network, spawn,
    /// tool_call, config_write, message.
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
            "command",
            "file_read",
            "file_write",
            "network",
            "spawn",
            "tool_call",
            "config_write",
            "message",
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

    /// Returns true if all eight permission dimensions are denied or absent.
    pub fn is_fully_denied(&self) -> bool {
        ![
            "command",
            "file_read",
            "file_write",
            "network",
            "spawn",
            "tool_call",
            "config_write",
            "message",
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
