//! Processor chain configuration and loader.
//!
//! This module provides types for deserializing processor chain configuration
//! from YAML/TOML and a [`ProcessorChainLoader`] that constructs a
//! [`ProcessorRegistry`] from that configuration.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;

use super::error::ProcessError;
use super::markdown_normalizer::MarkdownNormalizer;
use super::message_cleaner::MessageCleaner;
use super::processor::MessageProcessor;
use super::raw_log_processor::{RawLogConfig, RawLogProcessor};
use super::registry::ProcessorRegistry;

/// Configuration for the inbound processor chain.
///
/// Deserialized from the `processor_chain.inbound` section of the config file.
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessorChainConfig {
    /// Ordered list of processor configurations (execution order is by `priority`).
    #[serde(default)]
    pub inbound: Vec<ProcessorConfig>,
}

/// Configuration for a single processor.
///
/// Each variant corresponds to one concrete [`super::processor::MessageProcessor`]
/// implementation.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessorConfig {
    /// [`RawLogProcessor`](super::raw_log_processor::RawLogProcessor) — logs raw
    /// inbound messages to a JSON file.
    RawLog {
        /// Whether to write log files regardless of log level (default: `false`).
        #[serde(default)]
        enabled: bool,
        /// Directory to write log files into (default: `/tmp/processor_chain_logs`).
        #[serde(default = "default_log_dir")]
        dir: PathBuf,
        /// Number of days to retain log files (default: `7`).
        #[serde(default = "default_retention_days")]
        retention_days: u32,
    },
    /// [`MessageCleaner`](super::message_cleaner::MessageCleaner) — strips feishu
    /// platform fields and extracts clean text content.
    MessageCleaner,
    /// [`MarkdownNormalizer`](super::markdown_normalizer::MarkdownNormalizer) —
    /// standardises markdown formatting before LLM input.
    MarkdownNormalizer,
}

fn default_log_dir() -> PathBuf {
    PathBuf::from("/tmp/processor_chain_logs")
}

fn default_retention_days() -> u32 {
    7
}

/// Loads a [`ProcessorRegistry`] from a [`ProcessorChainConfig`].
///
/// Processors are instantiated according to their [`ProcessorConfig`] variant,
/// then registered to the registry in the order they appear in the config.
/// The registry sorts processors by ascending [`super::processor::MessageProcessor::priority`]
/// at execution time, so the config order does not need to match priority order.
pub struct ProcessorChainLoader;

impl ProcessorChainLoader {
    /// Constructs a [`ProcessorRegistry`] from `config`.
    ///
    /// Returns an empty registry when `config.inbound` is empty.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::ChainFailed`] if a processor cannot be constructed.
    pub fn load(config: &ProcessorChainConfig) -> Result<ProcessorRegistry, ProcessError> {
        let mut registry = ProcessorRegistry::new();
        for processor_config in &config.inbound {
            let processor = Self::build_processor(processor_config)?;
            registry.register(processor);
        }
        Ok(registry)
    }

    /// Builds a concrete processor from its configuration variant.
    fn build_processor(
        config: &ProcessorConfig,
    ) -> Result<Arc<dyn MessageProcessor>, ProcessError> {
        match config {
            ProcessorConfig::RawLog {
                enabled,
                dir,
                retention_days,
            } => {
                let cfg = RawLogConfig::new(*enabled, dir.clone(), *retention_days);
                let processor = RawLogProcessor::new(cfg).map_err(|e| {
                    ProcessError::chain_failed(format!("failed to create RawLogProcessor: {e}"))
                })?;
                Ok(Arc::new(processor))
            }
            ProcessorConfig::MessageCleaner => Ok(Arc::new(MessageCleaner::new())),
            ProcessorConfig::MarkdownNormalizer => Ok(Arc::new(MarkdownNormalizer::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_empty_config_returns_empty_registry() {
        let config = ProcessorChainConfig { inbound: vec![] };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 0);
    }

    #[test]
    fn test_load_raw_log_processor() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ProcessorChainConfig {
            inbound: vec![ProcessorConfig::RawLog {
                enabled: true,
                dir: tmp.path().to_path_buf(),
                retention_days: 7,
            }],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 1);
    }

    #[test]
    fn test_load_message_cleaner() {
        let config = ProcessorChainConfig {
            inbound: vec![ProcessorConfig::MessageCleaner],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 1);
    }

    #[test]
    fn test_load_markdown_normalizer() {
        let config = ProcessorChainConfig {
            inbound: vec![ProcessorConfig::MarkdownNormalizer],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 1);
    }

    #[test]
    fn test_load_all_three_processors() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ProcessorChainConfig {
            inbound: vec![
                ProcessorConfig::RawLog {
                    enabled: false,
                    dir: tmp.path().to_path_buf(),
                    retention_days: 7,
                },
                ProcessorConfig::MessageCleaner,
                ProcessorConfig::MarkdownNormalizer,
            ],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 3);
    }

    #[test]
    fn test_raw_log_config_deserialization() {
        let json = r#"{"type":"raw_log","enabled":true,"dir":"/tmp/logs","retention_days":14}"#;
        let config: ProcessorConfig = serde_json::from_str(json).unwrap();
        match config {
            ProcessorConfig::RawLog {
                enabled,
                dir,
                retention_days,
            } => {
                assert!(enabled);
                assert_eq!(dir, PathBuf::from("/tmp/logs"));
                assert_eq!(retention_days, 14);
            }
            _ => panic!("expected RawLog variant"),
        }
    }

    #[test]
    fn test_message_cleaner_deserialization() {
        let json = r#"{"type":"message_cleaner"}"#;
        let config: ProcessorConfig = serde_json::from_str(json).unwrap();
        match config {
            ProcessorConfig::MessageCleaner => {}
            _ => panic!("expected MessageCleaner variant"),
        }
    }

    #[test]
    fn test_markdown_normalizer_deserialization() {
        let json = r#"{"type":"markdown_normalizer"}"#;
        let config: ProcessorConfig = serde_json::from_str(json).unwrap();
        match config {
            ProcessorConfig::MarkdownNormalizer => {}
            _ => panic!("expected MarkdownNormalizer variant"),
        }
    }
}
