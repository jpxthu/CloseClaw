//! Processor chain configuration and loader.
//!
//! This module provides types for deserializing processor chain configuration
//! from YAML/TOML and a [`ProcessorChainLoader`] that constructs a
//! [`ProcessorRegistry`] from that configuration.

use std::path::PathBuf;

use serde::Deserialize;

use super::content_normalizer::ContentNormalizer;
use super::dsl_parser::DslParser;
use super::error::ProcessError;
use super::outbound_raw_log::OutboundRawLogProcessor;
use super::processor::MessageProcessor;
use super::raw_log_processor::{RawLogConfig, RawLogProcessor};
use super::registry::ProcessorRegistry;
use super::session_router::SessionRouter;

/// Configuration for the inbound processor chain.
///
/// Deserialized from the `processor_chain.inbound` section of the config file.
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessorChainConfig {
    /// Ordered list of inbound processor configurations.
    #[serde(default)]
    pub inbound: Vec<ProcessorConfig>,

    /// Ordered list of outbound processor configurations.
    #[serde(default)]
    pub outbound: Vec<ProcessorConfig>,
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
    /// [`ContentNormalizer`](super::content_normalizer::ContentNormalizer) — strips
    /// feishu platform fields, extracts clean text, and normalises markdown.
    ContentNormalizer,
    /// [`DslParser`](super::dsl_parser::DslParser) — outbound processor that
    /// parses `::button[...]` DSL instructions and stores them in metadata.
    DslParser,
    /// [`SessionRouter`](super::session_router::SessionRouter) — computes a
    /// deterministic `session_key` from routing fields.
    SessionRouter,
    /// [`OutboundRawLogProcessor`](super::outbound_raw_log::OutboundRawLogProcessor)
    /// — logs outbound messages to a JSON file.
    OutboundRawLog {
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
    /// Returns an empty registry when `config.inbound` and `config.outbound`
    /// are both empty.
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
        for processor_config in &config.outbound {
            let processor = Self::build_processor(processor_config)?;
            registry.register(processor);
        }
        Ok(registry)
    }

    /// Builds a concrete processor from its configuration variant.
    fn build_processor(
        config: &ProcessorConfig,
    ) -> Result<std::sync::Arc<dyn MessageProcessor>, ProcessError> {
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
                Ok(std::sync::Arc::new(processor))
            }
            ProcessorConfig::ContentNormalizer => Ok(std::sync::Arc::new(ContentNormalizer::new())),
            ProcessorConfig::SessionRouter => Ok(std::sync::Arc::new(SessionRouter::new())),
            ProcessorConfig::DslParser => Ok(std::sync::Arc::new(DslParser)),
            ProcessorConfig::OutboundRawLog {
                enabled,
                dir,
                retention_days,
            } => {
                let cfg = RawLogConfig::new(*enabled, dir.clone(), *retention_days);
                let processor = OutboundRawLogProcessor::new(cfg);
                Ok(std::sync::Arc::new(processor))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_empty_config_returns_empty_registry() {
        let config = ProcessorChainConfig {
            inbound: vec![],
            outbound: vec![],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 0);
        assert_eq!(registry.outbound_len(), 0);
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
            outbound: vec![],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 1);
    }

    #[test]
    fn test_load_content_normalizer() {
        let config = ProcessorChainConfig {
            inbound: vec![ProcessorConfig::ContentNormalizer],
            outbound: vec![],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 1);
    }

    #[test]
    fn test_load_all_processors() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ProcessorChainConfig {
            inbound: vec![
                ProcessorConfig::RawLog {
                    enabled: false,
                    dir: tmp.path().to_path_buf(),
                    retention_days: 7,
                },
                ProcessorConfig::ContentNormalizer,
            ],
            outbound: vec![],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 2);
    }

    #[test]
    fn test_load_dsl_parser_outbound() {
        let config = ProcessorChainConfig {
            inbound: vec![],
            outbound: vec![ProcessorConfig::DslParser],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.outbound_len(), 1);
    }

    #[test]
    fn test_load_both_inbound_and_outbound() {
        let config = ProcessorChainConfig {
            inbound: vec![ProcessorConfig::ContentNormalizer],
            outbound: vec![ProcessorConfig::DslParser],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 1);
        assert_eq!(registry.outbound_len(), 1);
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
    fn test_content_normalizer_deserialization() {
        let json = r#"{"type":"content_normalizer"}"#;
        let config: ProcessorConfig = serde_json::from_str(json).unwrap();
        match config {
            ProcessorConfig::ContentNormalizer => {}
            _ => panic!("expected ContentNormalizer variant"),
        }
    }

    #[test]
    fn test_dsl_parser_deserialization() {
        let json = r#"{"type":"dsl_parser"}"#;
        let config: ProcessorConfig = serde_json::from_str(json).unwrap();
        match config {
            ProcessorConfig::DslParser => {}
            _ => panic!("expected DslParser variant"),
        }
    }

    #[test]
    fn test_outbound_raw_log_deserialization() {
        let json =
            r#"{"type":"outbound_raw_log","enabled":true,"dir":"/tmp/out","retention_days":3}"#;
        let config: ProcessorConfig = serde_json::from_str(json).unwrap();
        match config {
            ProcessorConfig::OutboundRawLog {
                enabled,
                dir,
                retention_days,
            } => {
                assert!(enabled);
                assert_eq!(dir, PathBuf::from("/tmp/out"));
                assert_eq!(retention_days, 3);
            }
            _ => panic!("expected OutboundRawLog variant"),
        }
    }

    #[test]
    fn test_outbound_raw_log_defaults() {
        let json = r#"{"type":"outbound_raw_log"}"#;
        let config: ProcessorConfig = serde_json::from_str(json).unwrap();
        match config {
            ProcessorConfig::OutboundRawLog {
                enabled,
                dir,
                retention_days,
            } => {
                assert!(!enabled);
                assert_eq!(dir, PathBuf::from("/tmp/processor_chain_logs"));
                assert_eq!(retention_days, 7);
            }
            _ => panic!("expected OutboundRawLog variant"),
        }
    }

    #[test]
    fn test_load_outbound_raw_log_processor() {
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
    fn test_load_outbound_raw_log_with_others() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ProcessorChainConfig {
            inbound: vec![ProcessorConfig::ContentNormalizer],
            outbound: vec![
                ProcessorConfig::DslParser,
                ProcessorConfig::OutboundRawLog {
                    enabled: false,
                    dir: tmp.path().to_path_buf(),
                    retention_days: 14,
                },
            ],
        };
        let registry = ProcessorChainLoader::load(&config).unwrap();
        assert_eq!(registry.inbound_len(), 1);
        assert_eq!(registry.outbound_len(), 2);
    }
}
