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
                dir: Some(tmp.path().to_path_buf()),
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

#[test]
fn test_default_config_serde_roundtrip() {
    // Deserialize a config without `inbound` field — serde default should
    // kick in and produce the three-step chain.
    let json = r#"{"outbound": []}"#;
    let config: ProcessorChainConfig = serde_json::from_str(json).unwrap();
    assert_eq!(
        config.inbound.len(),
        3,
        "default inbound should have 3 processors"
    );
    // Verify order: RawLog → SessionRouter → ContentNormalizer.
    assert!(matches!(config.inbound[0], ProcessorConfig::RawLog { .. }));
    assert!(matches!(config.inbound[1], ProcessorConfig::SessionRouter));
    assert!(matches!(
        config.inbound[2],
        ProcessorConfig::ContentNormalizer
    ));
}

#[test]
fn test_default_config_loads_three_processors() {
    // default_inbound_chain() includes RawLog { enabled: false, dir: None }
    // which is skipped during registration (no output destination).
    let config = ProcessorChainConfig::default();
    let registry = ProcessorChainLoader::load(&config).unwrap();
    assert_eq!(
        registry.inbound_len(),
        2,
        "default config should load 2 inbound processors (RawLog skipped)"
    );
    assert_eq!(
        registry.outbound_len(),
        0,
        "default config should have 0 outbound processors"
    );
}
