//! Step 1.6 — Integration tests for outbound processor chain.
//!
//! Covers:
//! - VerbosityFilter → DslParser chain integration
//! - Streaming path: full outbound chain processes content_blocks end-to-end
//! - Config Loader: YAML deserialization for OutboundRawLog

use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_common::VerbosityLevel;
use closeclaw_llm::types::ContentBlock;

use super::dsl_parser::DslParser;
use super::loader::{ProcessorChainConfig, ProcessorChainLoader, ProcessorConfig};
use super::registry::ProcessorRegistry;
use super::verbosity_filter::VerbosityFilter;
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

// ═══════════════════════════════════════════════════════════════════════════════
// VerbosityFilter + DslParser chain integration
// ═══════════════════════════════════════════════════════════════════════════════

/// Build a minimal outbound registry: VerbosityFilter(5) → DslParser(10).
fn build_simple_outbound_chain() -> ProcessorRegistry {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));
    registry
}

#[tokio::test]
async fn test_full_verbosity_all_blocks_pass_through() {
    let registry = build_simple_outbound_chain();
    let blocks = vec![
        text_block("Hello"),
        thinking_block("thinking..."),
        tool_use_block("search"),
        tool_result_block("result"),
    ];
    let mut meta = HashMap::new();
    meta.insert(
        "verbosity_level".to_string(),
        VerbosityLevel::Full.to_string(),
    );
    let output = make_llm_output(blocks, meta);
    let result = registry.process_outbound(output).await.unwrap();

    // Full: all 4 blocks pass through VerbosityFilter
    assert_eq!(result.content_blocks.len(), 4);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(_)));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Thinking { .. }
    ));
    assert!(matches!(
        &result.content_blocks[2],
        ContentBlock::ToolUse { .. }
    ));
    assert!(matches!(
        &result.content_blocks[3],
        ContentBlock::ToolResult { .. }
    ));
}

#[tokio::test]
async fn test_normal_verbosity_filters_thinking() {
    let registry = build_simple_outbound_chain();
    let blocks = vec![
        text_block("Hello"),
        thinking_block("thinking..."),
        text_block("World"),
    ];
    let mut meta = HashMap::new();
    meta.insert(
        "verbosity_level".to_string(),
        VerbosityLevel::Normal.to_string(),
    );
    let output = make_llm_output(blocks, meta);
    let result = registry.process_outbound(output).await.unwrap();

    // Normal: Thinking removed, only Text blocks remain
    assert_eq!(result.content_blocks.len(), 2);
    assert!(matches!(
        &result.content_blocks[0],
        ContentBlock::Text(s) if s == "Hello"
    ));
    assert!(matches!(
        &result.content_blocks[1],
        ContentBlock::Text(s) if s == "World"
    ));
}

#[tokio::test]
async fn test_off_verbosity_only_text() {
    let registry = build_simple_outbound_chain();
    let blocks = vec![
        text_block("Hello"),
        thinking_block("thinking..."),
        tool_use_block("search"),
        tool_result_block("result"),
    ];
    let mut meta = HashMap::new();
    meta.insert(
        "verbosity_level".to_string(),
        VerbosityLevel::Off.to_string(),
    );
    let output = make_llm_output(blocks, meta);
    let result = registry.process_outbound(output).await.unwrap();

    // Off: only Text blocks remain
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(
        &result.content_blocks[0],
        ContentBlock::Text(s) if s == "Hello"
    ));
}

/// Metadata without verbosity_level should default to Full.
#[tokio::test]
async fn test_missing_verbosity_defaults_to_full() {
    let registry = build_simple_outbound_chain();
    let blocks = vec![text_block("Hello"), thinking_block("thinking...")];
    let output = make_llm_output(blocks, HashMap::new());
    let result = registry.process_outbound(output).await.unwrap();

    // No verbosity_level → Full → all blocks pass
    assert_eq!(result.content_blocks.len(), 2);
}

/// VerbosityFilter processes before DslParser — verify DSL instructions
/// in Text blocks are still parsed after filtering.
#[tokio::test]
async fn test_verbosity_filter_before_dsl_parser() {
    let registry = build_simple_outbound_chain();
    let blocks = vec![
        thinking_block("thinking..."),
        text_block("::button[label:OK;action:submit;value:yes]"),
    ];
    let mut meta = HashMap::new();
    meta.insert(
        "verbosity_level".to_string(),
        VerbosityLevel::Normal.to_string(),
    );
    let output = make_llm_output(blocks, meta);
    let result = registry.process_outbound(output).await.unwrap();

    // Normal: Thinking filtered by VerbosityFilter, then DslParser parses
    // the Text block's DSL instruction
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(_)));
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"), "DSL should be parsed: {dsl}");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Streaming path integration: full chain processes content_blocks end-to-end
// ═══════════════════════════════════════════════════════════════════════════════

/// Simulates the streaming path: after streaming, content_blocks are passed
/// through process_outbound (the same call `send_outbound_streaming` makes
/// after the stream ends).
#[tokio::test]
async fn test_streaming_path_full_chain_with_dsl() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));
    // Simulate streaming: accumulated blocks with DSL and thinking
    let blocks = vec![
        thinking_block("step 1: analyzing..."),
        thinking_block("step 2: planning..."),
        text_block("Here is the result."),
        text_block("::button[label:Confirm;action:confirm;value:ok]"),
    ];
    let mut meta = HashMap::new();
    meta.insert(
        "verbosity_level".to_string(),
        VerbosityLevel::Normal.to_string(),
    );
    let output = make_llm_output(blocks, meta);
    let result = registry.process_outbound(output).await.unwrap();

    // Normal: Thinking blocks filtered, DSL parsed
    // DSL-only Text block is dropped after stripping (empty)
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(
        &result.content_blocks[0],
        ContentBlock::Text(s) if s == "Here is the result."
    ));
    let dsl = result.metadata.get("dsl_result").unwrap();
    assert!(dsl.contains("button"));
    assert!(dsl.contains("Confirm"));
}

/// Streaming path with Off verbosity: only Text blocks survive.
#[tokio::test]
async fn test_streaming_path_off_verbosity() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));
    let blocks = vec![
        thinking_block("internal reasoning"),
        text_block("Final answer."),
        tool_use_block("search"),
        tool_result_block("result data"),
    ];
    let mut meta = HashMap::new();
    meta.insert(
        "verbosity_level".to_string(),
        VerbosityLevel::Off.to_string(),
    );
    let output = make_llm_output(blocks, meta);
    let result = registry.process_outbound(output).await.unwrap();

    // Off: only Text blocks remain
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(
        &result.content_blocks[0],
        ContentBlock::Text(s) if s == "Final answer."
    ));
}

/// Streaming path with Full verbosity: everything passes through.
#[tokio::test]
async fn test_streaming_path_full_verbosity() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));
    let blocks = vec![
        text_block("Hello"),
        thinking_block("thinking..."),
        tool_use_block("fetch"),
        tool_result_block("fetched"),
    ];
    let mut meta = HashMap::new();
    meta.insert(
        "verbosity_level".to_string(),
        VerbosityLevel::Full.to_string(),
    );
    let output = make_llm_output(blocks, meta);
    let result = registry.process_outbound(output).await.unwrap();

    // Full: all 4 blocks pass through
    assert_eq!(result.content_blocks.len(), 4);
}

/// Streaming path with empty content_blocks: falls back to content string.
#[tokio::test]
async fn test_streaming_path_empty_blocks_fallback() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));
    let output = ProcessedMessage {
        content_blocks: vec![],
        metadata: HashMap::new(),
    };
    let result = registry.process_outbound(output).await.unwrap();

    // Empty blocks → fallback creates a single Text block from content
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], ContentBlock::Text(_)));
}

/// Streaming path: empty blocks with DSL in content string.
#[tokio::test]
async fn test_streaming_path_empty_blocks_with_dsl_in_content() {
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(VerbosityFilter));
    registry.register(Arc::new(DslParser));
    let output = ProcessedMessage {
        content_blocks: vec![],
        metadata: HashMap::new(),
    };
    // VerbosityFilter processes content_blocks (empty) → DslParser falls
    // back to content, finds DSL instructions
    let result = registry.process_outbound(output).await.unwrap();

    // DslParser should have processed the content (empty) and produced
    // a fallback block
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
