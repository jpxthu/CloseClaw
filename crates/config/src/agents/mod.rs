//! Agent configuration module
//!
//! Provides AgentsConfigProvider for agents.json (registration list of agent IDs)
//! and AgentDirectoryProvider for loading agent configurations from directories.

mod config_types;
mod directory;
pub(crate) mod jsonc;
mod permission_provider;
mod provider;
mod resolved;
mod types;
mod validation;

// Re-export all config types from the local module.
pub use config_types::*;
pub use directory::AgentDirectoryProvider;
pub(crate) use jsonc::strip_jsonc_comments;
pub use permission_provider::{
    AgentPermissionProvider, LazyAgentPermissions, NoopPermissionProvider,
};
pub use provider::AgentsConfigProvider;
pub use resolved::{from_single, merge};
pub use types::AgentsConfig;
pub use validation::validate_agents_config;

// TEMPORARY (issue #3344): ResolvedAgentConfig / SubagentsConfig
// migrated to `closeclaw_common::agent_config`. This re-export keeps the
// legacy `closeclaw_config::agents::*` paths compiling while consumers are
// migrated one by one; removed in the wrap-up step (零二次出口终态).
pub use closeclaw_common::agent_config::{ResolvedAgentConfig, SubagentsConfig};

// TEMPORARY (issue #3344): the memory config type family migrated to
// `closeclaw_common::memory_config`. These re-exports keep the legacy
// `closeclaw_config::agents::*` paths compiling while consumers are
// migrated one by one; removed in the wrap-up step (零二次出口终态).
pub use closeclaw_common::memory_config::{
    default_capacity_max_rules, default_db_path, default_diary_path, default_dreaming_schedule,
    default_forgetting_initial_ttl_days, default_forgetting_injection_extension_days,
    default_forgetting_reidentify_extension_days, default_memory_md_path,
    default_mining_dedup_window_days, default_mining_max_events_per_session,
    default_scoring_cross_agent, default_scoring_explicitness, default_scoring_frequency,
    default_scoring_negative_signal, default_scoring_recency, default_search_context_turns,
    default_search_max_summary_chars, default_search_min_entity_hits, default_search_timeout_ms,
    default_search_top_k_events, default_threshold_absolute, default_threshold_relative,
    default_transcript_format, default_transcript_min_owner_msgs, default_transcript_min_turns,
    DreamingCapacityConfig, DreamingConfig, DreamingDiaryConfig, DreamingScoringConfig,
    DreamingThresholdConfig, ForgettingConfig, MemoryConfig, MemoryStorageConfig, MiningConfig,
    SearchConfig, TranscriptCleanRules,
};

#[cfg(test)]
mod directory_tests;

#[cfg(test)]
mod resolved_tests;

#[cfg(test)]
mod resolved_memory_tests;

#[cfg(test)]
mod permission_provider_tests;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
