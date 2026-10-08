//! Tests for the composition-root processor registry builder.

use crate::processor_registry::build_processor_chain;
use closeclaw_gateway::GatewayConfig;

fn config(raw_log_dir: Option<std::path::PathBuf>) -> GatewayConfig {
    GatewayConfig {
        name: "test-build-registry".to_string(),
        raw_log_dir,
        ..Default::default()
    }
}

/// Default config (no raw_log_dir): inbound = SessionRouter + ContentNormalizer,
/// outbound = VerbosityFilter + DslParser.
#[test]
fn test_build_processor_registry_default_chains() {
    let registry = crate::processor_registry::build_processor_registry(&config(None));
    assert_eq!(
        registry.inbound_len(),
        2,
        "default inbound chain must be SessionRouter + ContentNormalizer"
    );
    assert_eq!(
        registry.outbound_len(),
        2,
        "default outbound chain must be VerbosityFilter + DslParser"
    );
}

/// With `raw_log_dir` configured, the raw-log processors join both chains.
#[test]
fn test_build_processor_registry_raw_log_dir_adds_raw_log_processors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let raw_log_dir = Some(tmp.path().to_path_buf());
    let registry = crate::processor_registry::build_processor_registry(&config(raw_log_dir));
    assert_eq!(
        registry.inbound_len(),
        3,
        "raw_log_dir must add the inbound RawLogProcessor"
    );
    assert_eq!(
        registry.outbound_len(),
        3,
        "raw_log_dir must add the outbound OutboundRawLogProcessor"
    );
}

/// The chain handed to `Gateway::new` is a non-empty common `ProcessorChain`
/// trait object (the composition-root injection shape).
#[test]
fn test_build_processor_chain_is_injectable() {
    let chain = build_processor_chain(&config(None));
    assert_eq!((chain.inbound_len(), chain.outbound_len()), (2, 2));
}
