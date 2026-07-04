//! Step 1.3 — Integration tests for outbound processor chain.
//!
//! After Step 1.1/1.2, the outbound chain contains only DslParser.
//! Verbosity filtering and outbound logging are Gateway-level steps.
//!
//! Covers:
//! - DslParser-only outbound chain
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
