//! Step 1.3 — Integration tests for outbound processor chain.
//!
//! After Step 1.1/1.2, the outbound chain is:
//!   VerbosityFilter (5) → DslParser (10) → [OutboundRawLogProcessor (20)]
//!
//! Covers:
//! - VerbosityFilter + DslParser chain integration
//! - DslParser-only outbound chain (unit isolation)
//! - Config Loader: YAML deserialization for OutboundRawLog
//! - DslParser integration with various content block types

use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_llm::types::ContentBlock;

use super::dsl_parser::DslParser;
use super::loader::{ProcessorChainConfig, ProcessorChainLoader, ProcessorConfig};
use super::registry::ProcessorRegistry;
use super::ProcessedMessage;

// ── helpers ──────────────────────────────────────────────────────────────────

fn thinking_block(s: &str) -> ContentBlock {
    ContentBlock::Thinking {
        thinking: s.to_string(),
        signature: None,
    }
}

fn text_block(s: &str) -> ContentBlock {
    ContentBlock::Text(s.to_string())
}

fn tool_use_block(name: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: "call_1".to_string(),
        name: name.to_string(),
        input: "{}".to_string(),
    }
}

fn tool_result_block(content: &str) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_call_id: "call_1".to_string(),
        content: content.to_string(),
    }
}

fn make_llm_output(
    blocks: Vec<ContentBlock>,
    metadata: HashMap<String, String>,
) -> ProcessedMessage {
    ProcessedMessage {
        content_blocks: blocks,
        metadata,
    }
}

/// Build a minimal outbound registry: DslParser only.
fn build_dsl_only_outbound_chain() -> ProcessorRegistry {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(DslParser));
    registry
}

/// Build the full outbound registry: VerbosityFilter + DslParser.
/// Mirrors the chain produced by `build_processor_registry` for default config.
fn build_full_outbound_chain() -> ProcessorRegistry {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(super::verbosity_filter::VerbosityFilter));
    registry.register(Arc::new(DslParser));
    registry
}

/// Build a metadata map with the given verbosity level.
fn make_meta(verbosity: &str) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    m.insert("verbosity_level".to_string(), verbosity.to_string());
    m
}

// ═══════════════════════════════════════════════════════════════════════════════
// VerbosityFilter + DslParser chain integration
// ═══════════════════════════════════════════════════════════════════════════════

/// Full chain: Normal verbosity filters Thinking blocks, then DslParser processes remaining Text.
#[tokio::test]
async fn test_full_chain_normal_filters_thinking_then_parses_dsl() {
    let registry = build_full_outbound_chain();
    let blocks = vec![
        thinking_block("internal reasoning"),
        text_block("::button[label:OK;action:submit]"),
    ];
    let output = make_llm_output(blocks, make_meta("normal"));
    let result = registry.process_outbound(output).await.unwrap();

    // VerbosityFilter removes Thinking at Normal level
    // DslParser extracts DSL from remaining Text block
    assert_eq!(result.content_blocks.len(), 1);
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
}

/// Full chain: Off verbosity keeps only Text, then DslParser processes it.
#[tokio::test]
async fn test_full_chain_off_keeps_only_text_then_parses_dsl() {
    let registry = build_full_outbound_chain();
    let blocks = vec![
        thinking_block("rm"),
        text_block("Hello"),
        tool_use_block("search"),
    ];
    let output = make_llm_output(blocks, make_meta("off"));
    let result = registry.process_outbound(output).await.unwrap();

    // Off keeps only Text; DslParser passes through plain Text
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Hello"));
}

/// Full chain: Full verbosity keeps all blocks, then DslParser passes through.
#[tokio::test]
async fn test_full_chain_full_keeps_all_then_parses_dsl() {
    let registry = build_full_outbound_chain();
    let blocks = vec![
        text_block("Hello"),
        thinking_block("reasoning"),
        tool_use_block("search"),
    ];
    let output = make_llm_output(blocks, make_meta("full"));
    let result = registry.process_outbound(output).await.unwrap();

    // Full keeps all blocks; DslParser passes through non-DSL Text
    assert_eq!(result.content_blocks.len(), 3);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Hello"));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Thinking { .. }
    ));
    assert!(matches!(
        &result.content_blocks[2],
        ContentBlock::ToolUse { .. }
    ));
}

/// Full chain: empty blocks — VerbosityFilter creates fallback, DslParser processes it.
#[tokio::test]
async fn test_full_chain_empty_blocks() {
    let registry = build_full_outbound_chain();
    let output = make_llm_output(vec![], make_meta("normal"));
    let result = registry.process_outbound(output).await.unwrap();

    // Fallback creates a single Text block from content (empty)
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(_)));
}

/// Full chain: mixed blocks with DSL — Normal filters Thinking, DslParser extracts DSL.
#[tokio::test]
async fn test_full_chain_mixed_blocks_with_dsl() {
    let registry = build_full_outbound_chain();
    let blocks = vec![
        thinking_block("step 1"),
        text_block("Result here."),
        text_block("::button[label:OK;action:ok]"),
        tool_use_block("tool"),
    ];
    let output = make_llm_output(blocks, make_meta("normal"));
    let result = registry.process_outbound(output).await.unwrap();

    // Normal: Thinking filtered; DslParser: first Text kept, second Text (DSL) stripped, ToolUse passed
    assert_eq!(result.content_blocks.len(), 2);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Result here."));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::ToolUse { .. }
    ));
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
}

/// Full chain: missing verbosity_level in metadata defaults to Full.
#[tokio::test]
async fn test_full_chain_missing_verbosity_defaults_to_normal() {
    let registry = build_full_outbound_chain();
    let blocks = vec![thinking_block("reasoning"), text_block("Hello")];
    let output = make_llm_output(blocks, HashMap::new());
    let result = registry.process_outbound(output).await.unwrap();

    // No verbosity_level → Normal (default) → Thinking filtered
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(_)));
}

/// Full chain: Normal verbosity with DSL in mixed text — verifies ordering.
#[tokio::test]
async fn test_full_chain_normal_ordering() {
    let registry = build_full_outbound_chain();
    let blocks = vec![
        thinking_block("internal"),
        text_block("::button[label:A;action:a]"),
        thinking_block("internal2"),
        text_block("Plain text"),
    ];
    let output = make_llm_output(blocks, make_meta("normal"));
    let result = registry.process_outbound(output).await.unwrap();

    // Both Thinking blocks filtered; first Text (DSL-only) stripped; second Text kept
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "Plain text"));
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// DslParser-only outbound chain
// ═══════════════════════════════════════════════════════════════════════════════

/// Plain text passes through DslParser unchanged.
#[tokio::test]
async fn test_dsl_parser_only_plain_text() {
    let registry = build_dsl_only_outbound_chain();
    let blocks = vec![text_block("Hello world")];
    let output = make_llm_output(blocks, HashMap::new());
    let result = registry.process_outbound(output).await.unwrap();

    assert_eq!(result.content_blocks.len(), 1);
    assert_eq!(result.text_content(), Some("Hello world"));
}

/// DslParser parses DSL instructions from Text blocks.
/// When a DSL-only block is processed, DslParser extracts the DSL and
/// falls back to the original content string (preserving the text).
#[tokio::test]
async fn test_dsl_parser_extracts_dsl() {
    let registry = build_dsl_only_outbound_chain();
    let blocks = vec![text_block("::button[label:OK;action:submit;value:yes]")];
    let output = make_llm_output(blocks, HashMap::new());
    let result = registry.process_outbound(output).await.unwrap();

    // DSL-only block: DslParser extracts DSL, fallback keeps original text
    assert_eq!(result.content_blocks.len(), 1);
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"), "DSL should be parsed: {dsl}");
}

/// Mixed content: text + DSL in same block.
#[tokio::test]
async fn test_dsl_parser_mixed_content() {
    let registry = build_dsl_only_outbound_chain();
    let blocks = vec![text_block(
        "Here is the result.\n::button[label:OK;action:submit;value:yes]",
    )];
    let output = make_llm_output(blocks, HashMap::new());
    let result = registry.process_outbound(output).await.unwrap();

    // Text before DSL is kept, DSL is extracted
    assert_eq!(result.content_blocks.len(), 1);
    assert!(
        matches!(&result.content_blocks[0], ContentBlock::Text(s) if s.contains("Here is the result."))
    );
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
}

/// Non-Text blocks pass through DslParser unchanged.
#[tokio::test]
async fn test_dsl_parser_non_text_blocks_passthrough() {
    let registry = build_dsl_only_outbound_chain();
    let blocks = vec![
        thinking_block("reasoning..."),
        tool_use_block("search"),
        tool_result_block("result"),
    ];
    let output = make_llm_output(blocks, HashMap::new());
    let result = registry.process_outbound(output).await.unwrap();

    // DslParser only processes Text blocks; non-Text pass through
    assert_eq!(result.content_blocks.len(), 3);
    assert!(matches!(
        &result.content_blocks[0],
        ContentBlock::Thinking { .. }
    ));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::ToolUse { .. }
    ));
    assert!(matches!(
        &result.content_blocks[2],
        ContentBlock::ToolResult { .. }
    ));
}

/// Empty content_blocks: DslParser falls back to content string.
#[tokio::test]
async fn test_dsl_parser_empty_blocks_fallback() {
    let registry = build_dsl_only_outbound_chain();
    let output = ProcessedMessage {
        content_blocks: vec![],
        metadata: HashMap::new(),
    };
    let result = registry.process_outbound(output).await.unwrap();

    // Fallback creates a single Text block from content (empty)
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(_)));
}

/// Multiple Text blocks: DSL parsed from each.
#[tokio::test]
async fn test_dsl_parser_multiple_text_blocks() {
    let registry = build_dsl_only_outbound_chain();
    let blocks = vec![
        text_block("First paragraph."),
        text_block("::button[label:Next;action:next;value:ok]"),
        text_block("Third paragraph."),
    ];
    let output = make_llm_output(blocks, HashMap::new());
    let result = registry.process_outbound(output).await.unwrap();

    // Two non-DSL text blocks remain, DSL block stripped
    assert_eq!(result.content_blocks.len(), 2);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(s) if s == "First paragraph."));
    assert!(matches!(&result.content_blocks[1], ContentBlock::Text(s) if s == "Third paragraph."));
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// Streaming path integration: DslParser processes accumulated blocks
// ═══════════════════════════════════════════════════════════════════════════════

/// Simulates streaming path: accumulated blocks processed by DslParser only.
#[tokio::test]
async fn test_streaming_path_dsl_parser_only() {
    let registry = build_dsl_only_outbound_chain();
    let blocks = vec![
        thinking_block("step 1: analyzing..."),
        thinking_block("step 2: planning..."),
        text_block("Here is the result."),
        text_block("::button[label:Confirm;action:confirm;value:ok]"),
    ];
    let output = make_llm_output(blocks, HashMap::new());
    let result = registry.process_outbound(output).await.unwrap();

    // DslParser processes Text blocks; thinking/tool blocks pass through
    // DSL-only Text block is dropped after stripping
    assert_eq!(result.content_blocks.len(), 3); // 2 thinking + 1 text
    assert!(matches!(
        &result.content_blocks[0],
        ContentBlock::Thinking { .. }
    ));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Thinking { .. }
    ));
    assert!(matches!(
        &result.content_blocks[2],
        ContentBlock::Text(s) if s == "Here is the result."
    ));
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
    assert!(dsl.contains("Confirm"));
}

/// Streaming path with DSL in content fallback.
#[tokio::test]
async fn test_streaming_path_empty_blocks_with_dsl_in_content() {
    let registry = build_dsl_only_outbound_chain();
    let output = ProcessedMessage {
        content_blocks: vec![],
        metadata: HashMap::new(),
    };
    let result = registry.process_outbound(output).await.unwrap();

    // Empty blocks → fallback creates a single Text block from content (empty)
    assert_eq!(result.content_blocks.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Config Loader — OutboundRawLog deserialization and loading
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_outbound_raw_log_yaml_deserialization() {
    let yaml = r#"
type: outbound_raw_log
enabled: true
dir: /var/log/outbound
retention_days: 30
"#;
    let config: ProcessorConfig = serde_yaml::from_str(yaml).unwrap();
    match config {
        ProcessorConfig::OutboundRawLog {
            enabled,
            dir,
            retention_days,
        } => {
            assert!(enabled);
            assert_eq!(dir, std::path::PathBuf::from("/var/log/outbound"));
            assert_eq!(retention_days, 30);
        }
        other => panic!("expected OutboundRawLog variant, got: {other:?}"),
    }
}

#[test]
fn test_outbound_raw_log_yaml_defaults() {
    let yaml = r#"
type: outbound_raw_log
"#;
    let config: ProcessorConfig = serde_yaml::from_str(yaml).unwrap();
    match config {
        ProcessorConfig::OutboundRawLog {
            enabled,
            dir,
            retention_days,
        } => {
            assert!(!enabled);
            assert_eq!(dir, std::path::PathBuf::from("/tmp/processor_chain_logs"));
            assert_eq!(retention_days, 7);
        }
        other => panic!("expected OutboundRawLog variant, got: {other:?}"),
    }
}

#[test]
fn test_full_config_yaml_with_outbound_raw_log() {
    let yaml = r#"
inbound:
  - type: content_normalizer
outbound:
  - type: dsl_parser
  - type: outbound_raw_log
    enabled: true
    dir: /tmp/logs
    retention_days: 14
"#;
    let config: ProcessorChainConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.inbound.len(), 1);
    assert_eq!(config.outbound.len(), 2);
}

#[test]
fn test_config_loader_loads_outbound_raw_log() {
    let tmp = tempfile::tempdir().unwrap();
    let config = ProcessorChainConfig {
        inbound: vec![],
        outbound: vec![ProcessorConfig::OutboundRawLog {
            enabled: true,
            dir: tmp.path().to_path_buf(),
            retention_days: 5,
        }],
    };
    let registry = ProcessorChainLoader::load(&config).unwrap();
    assert_eq!(registry.outbound_len(), 1);
}

#[test]
fn test_config_loader_outbound_raw_log_with_all_types() {
    let tmp = tempfile::tempdir().unwrap();
    let config = ProcessorChainConfig {
        inbound: vec![ProcessorConfig::ContentNormalizer],
        outbound: vec![
            ProcessorConfig::DslParser,
            ProcessorConfig::OutboundRawLog {
                enabled: true,
                dir: tmp.path().to_path_buf(),
                retention_days: 10,
            },
        ],
    };
    let registry = ProcessorChainLoader::load(&config).unwrap();
    assert_eq!(registry.inbound_len(), 1);
    assert_eq!(registry.outbound_len(), 2);
}

/// End-to-end: build a registry from config, process outbound message through it.
#[tokio::test]
async fn test_config_loader_end_to_end_outbound() {
    let tmp = tempfile::tempdir().unwrap();
    let config = ProcessorChainConfig {
        inbound: vec![],
        outbound: vec![
            ProcessorConfig::DslParser,
            ProcessorConfig::OutboundRawLog {
                enabled: true,
                dir: tmp.path().to_path_buf(),
                retention_days: 7,
            },
        ],
    };
    let registry = ProcessorChainLoader::load(&config).unwrap();

    let output = ProcessedMessage {
        content_blocks: vec![text_block("Test output")],
        metadata: HashMap::new(),
    };
    let result = registry.process_outbound(output).await.unwrap();
    assert_eq!(result.text_content(), Some("Test output"));
}
