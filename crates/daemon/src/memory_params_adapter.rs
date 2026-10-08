//! Adapter: field-level mapping from config memory types onto the memory
//! crate's self-held parameter types.
//!
//! `closeclaw-memory` does not depend on `closeclaw-config`; the
//! composition layer (daemon) maps resolved config values onto
//! memory-owned params at assembly time. Default-value fallbacks live in
//! the memory crate (`closeclaw_memory::params`), so this module performs
//! pure field copies — no defaulting logic here.

use closeclaw_config::agents::{DreamingConfig, MemoryConfig};
use closeclaw_memory::params::{
    DreamingCapacityParams, DreamingDiaryParams, DreamingParams, DreamingScoringParams,
    DreamingThresholdParams, MinerParams, TranscriptCleanRules,
};

/// Map the resolved memory config onto miner [`MinerParams`].
///
/// Aggregates `mining.*` plus the miner-consumed `forgetting.*` fields
/// (`initial_ttl_days`, `reidentify_extension_days`).
pub(crate) fn miner_params_from_memory(config: &MemoryConfig) -> MinerParams {
    MinerParams {
        enabled: config.mining.enabled,
        model: config.mining.model.clone(),
        max_events_per_session: config.mining.max_events_per_session,
        dedup_window_days: config.mining.dedup_window_days,
        clean_rules: TranscriptCleanRules {
            min_turns: config.mining.transcript_clean_rules.min_turns,
            min_owner_msgs: config.mining.transcript_clean_rules.min_owner_msgs,
            format: config.mining.transcript_clean_rules.format.clone(),
        },
        initial_ttl_days: config.forgetting.initial_ttl_days,
        reidentify_extension_days: config.forgetting.reidentify_extension_days,
    }
}

/// Map the resolved memory config onto a miner [`MinerConfig`].
pub(crate) fn miner_config_from_memory(
    config: &MemoryConfig,
) -> closeclaw_memory::miner::MinerConfig {
    closeclaw_memory::miner::MinerConfig::from(miner_params_from_memory(config))
}

/// Map the resolved dreaming config onto [`DreamingParams`].
pub(crate) fn dreaming_params_from_config(config: &DreamingConfig) -> DreamingParams {
    DreamingParams {
        enabled: config.enabled,
        diary: DreamingDiaryParams {
            enabled: config.diary.enabled,
            path: config.diary.path.clone(),
        },
        model: config.model.clone(),
        scoring: DreamingScoringParams {
            frequency_weight: config.scoring.frequency_weight,
            recency_weight: config.scoring.recency_weight,
            explicitness_weight: config.scoring.explicitness_weight,
            cross_agent_weight: config.scoring.cross_agent_weight,
            negative_signal_weight: config.scoring.negative_signal_weight,
        },
        threshold: DreamingThresholdParams {
            absolute: config.threshold.absolute,
            relative: config.threshold.relative,
        },
        capacity: DreamingCapacityParams {
            max_rules: config.capacity.max_rules,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_config::agents::{ForgettingConfig, MiningConfig};
    use closeclaw_memory::params::{
        default_forgetting_initial_ttl_days, default_forgetting_reidentify_extension_days,
        default_mining_dedup_window_days, default_mining_max_events_per_session,
    };

    #[test]
    fn test_miner_config_from_memory_maps_mining_fields() {
        let config = MemoryConfig {
            mining: MiningConfig {
                enabled: Some(true),
                model: Some("gpt-4o-mini".to_string()),
                max_events_per_session: Some(20),
                dedup_window_days: Some(14),
                transcript_clean_rules: closeclaw_config::agents::TranscriptCleanRules {
                    min_turns: Some(3),
                    min_owner_msgs: Some(4),
                    format: Some("plain".to_string()),
                },
            },
            ..Default::default()
        };

        let miner = miner_config_from_memory(&config);
        assert!(miner.enabled);
        assert_eq!(miner.model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(miner.max_events_per_session, 20);
        assert_eq!(miner.dedup_window_days, 14);
        assert_eq!(miner.clean_rules.min_turns, Some(3));
        assert_eq!(miner.clean_rules.min_owner_msgs, Some(4));
        assert_eq!(miner.clean_rules.format.as_deref(), Some("plain"));
    }

    #[test]
    fn test_miner_config_from_memory_maps_forgetting_fields() {
        let config = MemoryConfig {
            forgetting: ForgettingConfig {
                initial_ttl_days: Some(60),
                reidentify_extension_days: Some(45),
                injection_extension_days: Some(10),
            },
            ..Default::default()
        };

        let miner = miner_config_from_memory(&config);
        assert_eq!(miner.initial_ttl_days, 60);
        assert_eq!(miner.reidentify_extension_days, 45);
    }

    #[test]
    fn test_miner_config_from_memory_defaults_fallback() {
        let miner = miner_config_from_memory(&MemoryConfig::default());
        assert!(
            !miner.enabled,
            "mining.enabled missing → miner disabled (default false)"
        );
        assert_eq!(
            miner.max_events_per_session,
            default_mining_max_events_per_session() as usize
        );
        assert_eq!(miner.dedup_window_days, default_mining_dedup_window_days());
        assert_eq!(
            miner.initial_ttl_days,
            default_forgetting_initial_ttl_days()
        );
        assert_eq!(
            miner.reidentify_extension_days,
            default_forgetting_reidentify_extension_days()
        );
    }

    #[test]
    fn test_dreaming_params_from_config_maps_all_fields() {
        let config = DreamingConfig {
            enabled: Some(true),
            diary: closeclaw_config::agents::DreamingDiaryConfig {
                enabled: Some(false),
                path: Some("custom/diary/".to_string()),
            },
            model: Some("dream-model".to_string()),
            schedule: Some("30 4 * * *".to_string()),
            scoring: closeclaw_config::agents::DreamingScoringConfig {
                frequency_weight: Some(2.0),
                recency_weight: Some(0.7),
                explicitness_weight: Some(1.1),
                cross_agent_weight: Some(1.4),
                negative_signal_weight: Some(-0.8),
            },
            threshold: closeclaw_config::agents::DreamingThresholdConfig {
                absolute: Some(3.0),
                relative: Some(0.5),
            },
            capacity: closeclaw_config::agents::DreamingCapacityConfig { max_rules: Some(8) },
        };

        let params = dreaming_params_from_config(&config);
        assert_eq!(params.enabled, Some(true));
        assert_eq!(params.diary.enabled, Some(false));
        assert_eq!(params.diary.path.as_deref(), Some("custom/diary/"));
        assert_eq!(params.model.as_deref(), Some("dream-model"));
        assert_eq!(params.scoring.frequency_weight, Some(2.0));
        assert_eq!(params.scoring.recency_weight, Some(0.7));
        assert_eq!(params.scoring.explicitness_weight, Some(1.1));
        assert_eq!(params.scoring.cross_agent_weight, Some(1.4));
        assert_eq!(params.scoring.negative_signal_weight, Some(-0.8));
        assert_eq!(params.threshold.absolute, Some(3.0));
        assert_eq!(params.threshold.relative, Some(0.5));
        assert_eq!(params.capacity.max_rules, Some(8));
    }

    #[test]
    fn test_dreaming_params_from_config_defaults_stay_none() {
        let params = dreaming_params_from_config(&DreamingConfig::default());
        assert_eq!(params.enabled, None);
        assert_eq!(params.diary.enabled, None);
        assert_eq!(params.diary.path, None);
        assert_eq!(params.model, None);
        assert_eq!(params.scoring, DreamingScoringParams::default());
        assert_eq!(params.threshold, DreamingThresholdParams::default());
        assert_eq!(params.capacity, DreamingCapacityParams::default());
    }
}
