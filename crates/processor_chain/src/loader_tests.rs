//! Unit tests for [`ProcessorChainLoader`] — SessionRouter variant coverage.

use super::*;

#[test]
fn test_session_router_serde_roundtrip() {
    let json = r#"{"type":"session_router"}"#;
    let config: ProcessorConfig = serde_json::from_str(json).unwrap();
    match config {
        ProcessorConfig::SessionRouter => {}
        _ => panic!("expected SessionRouter variant"),
    }
}

#[test]
fn test_load_full_inbound_chain_with_session_router() {
    let tmp = tempfile::tempdir().unwrap();
    let config = ProcessorChainConfig {
        inbound: vec![
            ProcessorConfig::RawLog {
                enabled: true,
                dir: tmp.path().to_path_buf(),
                retention_days: 7,
            },
            ProcessorConfig::SessionRouter,
            ProcessorConfig::ContentNormalizer,
        ],
        outbound: vec![],
    };
    let registry = ProcessorChainLoader::load(&config).unwrap();
    assert_eq!(
        registry.inbound_len(),
        3,
        "full inbound chain should contain 3 processors"
    );
}

#[test]
fn test_load_session_router_alone() {
    let config = ProcessorChainConfig {
        inbound: vec![ProcessorConfig::SessionRouter],
        outbound: vec![],
    };
    let registry = ProcessorChainLoader::load(&config).unwrap();
    assert_eq!(
        registry.inbound_len(),
        1,
        "single SessionRouter should result in inbound_len == 1"
    );
}
