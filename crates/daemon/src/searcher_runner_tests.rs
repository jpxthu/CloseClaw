//! Unit tests for the composition-root active-searcher pipeline assembly.
//!
//! Covers the two dependency-alignment dimensions relocated here from the
//! gateway during the memory assembly move-up:
//! - config → memory params field mapping (search / forgetting params,
//!   `build_searcher_config` defaults and enable gate, config round trip);
//! - conversion between session message snapshots / memory injection
//!   payloads and the tuple protocol of the [`SearcherRunner`](closeclaw_gateway::SearcherRunner)
//!   seam (the slot half of the conversion lives in the gateway and is
//!   cross-checked here by the round-trip test).

use std::collections::HashSet;

use super::{
    build_searcher_config, deserialize_memory_config, forgetting_params_from_memory_config,
    search_params_from_memory_config, snapshots_to_internal_messages, summary_to_slot_parts,
};
use closeclaw_common::{ForgettingConfig, MemoryConfig, SearchConfig};
use closeclaw_memory::params::{
    default_forgetting_injection_extension_days, default_search_context_turns,
    default_search_max_summary_chars, default_search_min_entity_hits, default_search_timeout_ms,
    default_search_top_k_events,
};

fn full_search_config() -> SearchConfig {
    SearchConfig {
        enabled: Some(true),
        model: Some("search-model".to_string()),
        context_turns: Some(8),
        timeout_ms: Some(1500),
        max_summary_chars: Some(300),
        min_entity_hits: Some(2),
        top_k_events: Some(4),
    }
}

// ── Config → memory params mapping ────────────────────────────────────────

#[test]
fn test_search_params_mapping_field_equivalence() {
    let mem_cfg = MemoryConfig {
        search: full_search_config(),
        ..Default::default()
    };
    let params = search_params_from_memory_config(&mem_cfg);
    assert_eq!(params.enabled, Some(true));
    assert_eq!(params.model.as_deref(), Some("search-model"));
    assert_eq!(params.context_turns, Some(8));
    assert_eq!(params.timeout_ms, Some(1500));
    assert_eq!(params.max_summary_chars, Some(300));
    assert_eq!(params.min_entity_hits, Some(2));
    assert_eq!(params.top_k_events, Some(4));
}

#[test]
fn test_forgetting_params_mapping_field_equivalence() {
    let mem_cfg = MemoryConfig {
        forgetting: ForgettingConfig {
            injection_extension_days: Some(21),
            ..Default::default()
        },
        ..Default::default()
    };
    let params = forgetting_params_from_memory_config(&mem_cfg);
    assert_eq!(
        params.injection_extension_days,
        Some(21),
        "the searcher-consumed forgetting field must be carried over"
    );
}

#[test]
fn test_mapping_with_missing_fields_stays_none() {
    let params = search_params_from_memory_config(&MemoryConfig::default());
    assert_eq!(
        params,
        Default::default(),
        "undeclared config fields must stay None; the memory crate owns the default fallbacks"
    );
    let forgetting = forgetting_params_from_memory_config(&MemoryConfig::default());
    assert_eq!(forgetting.injection_extension_days, None);
}

#[test]
fn test_build_searcher_config_applies_memory_defaults_for_missing_fields() {
    let mem_cfg = MemoryConfig {
        search: SearchConfig {
            enabled: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    let config = build_searcher_config("agent-model", &Some(mem_cfg))
        .expect("enabled search must build a config");
    assert_eq!(config.timeout_ms, default_search_timeout_ms());
    assert_eq!(config.max_summary_chars, default_search_max_summary_chars());
    assert_eq!(config.min_entity_hits, default_search_min_entity_hits());
    assert_eq!(config.top_k_events, default_search_top_k_events());
    assert_eq!(config.context_turns, default_search_context_turns());
    assert_eq!(
        config.injection_extension_days,
        default_forgetting_injection_extension_days()
    );
    assert_eq!(
        config.model, "agent-model",
        "missing search.model must fall back to the agent model"
    );
}

#[test]
fn test_build_searcher_config_carries_explicit_overrides() {
    let mem_cfg = MemoryConfig {
        search: full_search_config(),
        forgetting: ForgettingConfig {
            injection_extension_days: Some(14),
            ..Default::default()
        },
        ..Default::default()
    };
    let config = build_searcher_config("agent-model", &Some(mem_cfg))
        .expect("enabled search must build a config");
    assert_eq!(
        config.model, "search-model",
        "explicit search.model wins over the agent model"
    );
    assert_eq!(config.timeout_ms, 1500);
    assert_eq!(config.context_turns, 8);
    assert_eq!(config.injection_extension_days, 14);
}

#[test]
fn test_build_searcher_config_none_when_search_disabled() {
    let mem_cfg = MemoryConfig {
        search: SearchConfig {
            enabled: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        build_searcher_config("agent-model", &Some(mem_cfg)).is_none(),
        "explicitly disabled search must not build a searcher config"
    );
}

#[test]
fn test_build_searcher_config_without_memory_config_uses_defaults() {
    let config = build_searcher_config("agent-model", &None)
        .expect("absent memory config keeps search enabled with defaults");
    assert_eq!(config.timeout_ms, default_search_timeout_ms());
    assert_eq!(config.context_turns, default_search_context_turns());
    assert_eq!(
        config.injection_extension_days,
        default_forgetting_injection_extension_days()
    );
    assert_eq!(config.model, "agent-model");
}

#[test]
fn test_deserialize_memory_config_round_trip() {
    let mem_cfg = MemoryConfig {
        search: full_search_config(),
        ..Default::default()
    };
    let json = serde_json::to_value(&mem_cfg).expect("serialize memory config");
    let parsed = deserialize_memory_config(&json).expect("serialized config must deserialize");
    assert_eq!(
        parsed.search,
        full_search_config(),
        "serde round trip must preserve the search fields"
    );
}

// ── Snapshot → internal message conversion ────────────────────────────────

fn snapshot(
    role: &str,
    content: &str,
) -> closeclaw_session::active_searcher::SessionMessageSnapshot {
    closeclaw_session::active_searcher::SessionMessageSnapshot {
        role: role.to_string(),
        content: content.to_string(),
    }
}

#[test]
fn test_snapshots_map_with_field_equivalence() {
    use closeclaw_common::llm_types::InternalMessage;
    use closeclaw_common::processor::ContentBlock;

    let snapshots = vec![
        snapshot("user", "hello there"),
        snapshot("assistant", "hi, how can I help?"),
    ];
    let messages = snapshots_to_internal_messages(&snapshots);
    assert_eq!(messages.len(), 2, "one internal message per snapshot");
    let expected: Vec<InternalMessage> = snapshots
        .iter()
        .map(|m| InternalMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            content_blocks: Some(vec![ContentBlock::Text(m.content.clone())]),
            tool_call_id: None,
        })
        .collect();
    assert_eq!(messages, expected, "role/content/blocks must map 1:1");
}

#[test]
fn test_empty_snapshot_list_maps_to_empty_messages() {
    let messages = snapshots_to_internal_messages(&[]);
    assert!(messages.is_empty(), "no snapshots must yield no messages");
}

// ── Summary → tuple parts conversion ──────────────────────────────────────

fn summary(
    content: &str,
    position: closeclaw_memory::active_searcher::MemorySummaryPosition,
    event_ids: &[i64],
) -> closeclaw_memory::active_searcher::InjectedMemorySummary {
    closeclaw_memory::active_searcher::InjectedMemorySummary {
        content: content.to_string(),
        position,
        injected_event_ids: event_ids.iter().copied().collect(),
    }
}

#[test]
fn test_before_next_position_maps_to_before_next_tag() {
    use closeclaw_memory::active_searcher::MemorySummaryPosition;

    let (content, tag, ids) = summary_to_slot_parts(summary(
        "remembered fact",
        MemorySummaryPosition::BeforeNext,
        &[7, 9],
    ));
    assert_eq!(content, "remembered fact");
    assert_eq!(tag, "before_next");
    assert_eq!(ids, HashSet::from([7, 9]), "event ids pass through");
}

#[test]
fn test_after_current_position_maps_to_after_current_tag() {
    use closeclaw_memory::active_searcher::MemorySummaryPosition;

    let (_, tag, ids) = summary_to_slot_parts(summary(
        "other fact",
        MemorySummaryPosition::AfterCurrent,
        &[3],
    ));
    assert_eq!(tag, "after_current");
    assert_eq!(ids, HashSet::from([3]));
}

// ── Round trip: summary → parts → session injection ───────────────────────

#[test]
fn test_summary_to_session_injection_round_trip_is_lossless() {
    use closeclaw_gateway::injection_convert::slot_parts_to_session_injection;
    use closeclaw_memory::active_searcher::MemorySummaryPosition;
    use closeclaw_session::llm_session::InjectionPosition;

    for (position, tag) in [
        (MemorySummaryPosition::BeforeNext, "before_next"),
        (MemorySummaryPosition::AfterCurrent, "after_current"),
    ] {
        let payload = summary("round trip", position, &[11, 12]);
        let (content, position_tag, ids) = summary_to_slot_parts(payload);
        let injection = slot_parts_to_session_injection(content, &position_tag, ids);
        assert_eq!(injection.content, "round trip");
        let expected_mode = match position {
            MemorySummaryPosition::BeforeNext => InjectionPosition::BeforeNext,
            MemorySummaryPosition::AfterCurrent => InjectionPosition::AfterCurrent,
        };
        assert_eq!(
            injection.position_mode, expected_mode,
            "position semantics must survive the round trip via tag {tag}"
        );
        assert_eq!(
            injection.injected_event_ids,
            HashSet::from([11, 12]),
            "injected event ids must survive the round trip"
        );
    }
}
