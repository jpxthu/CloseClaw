//! Unit tests for [`ProcessorChainLoader`] — SessionRouter variant coverage.

use super::*;
use closeclaw_gateway::DmScope;

#[test]
fn test_session_router_serde_roundtrip() {
    let json = r#"{"type":"session_router","dm_scope":"per-account-channel-peer"}"#;
    let config: ProcessorConfig = serde_json::from_str(json).unwrap();
    match config {
        ProcessorConfig::SessionRouter { dm_scope } => {
            assert_eq!(dm_scope, DmScope::PerAccountChannelPeer);
        }
        _ => panic!("expected SessionRouter variant"),
    }
}

#[test]
fn test_session_router_default_dm_scope() {
    let json = r#"{"type":"session_router"}"#;
    let config: ProcessorConfig = serde_json::from_str(json).unwrap();
    match config {
        ProcessorConfig::SessionRouter { dm_scope } => {
            assert_eq!(
                dm_scope,
                DmScope::PerAccountChannelPeer,
                "omitted dm_scope should default to PerAccountChannelPeer"
            );
        }
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
            ProcessorConfig::SessionRouter {
                dm_scope: DmScope::PerChannelPeer,
            },
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
        inbound: vec![ProcessorConfig::SessionRouter {
            dm_scope: DmScope::Main,
        }],
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
fn test_session_router_serde_all_scopes() {
    let scopes = [
        (
            r#"{"type":"session_router","dm_scope":"main"}"#,
            DmScope::Main,
        ),
        (
            r#"{"type":"session_router","dm_scope":"per-peer"}"#,
            DmScope::PerPeer,
        ),
        (
            r#"{"type":"session_router","dm_scope":"per-channel-peer"}"#,
            DmScope::PerChannelPeer,
        ),
        (
            r#"{"type":"session_router","dm_scope":"per-account-channel-peer"}"#,
            DmScope::PerAccountChannelPeer,
        ),
        (
            r#"{"type":"session_router","dm_scope":"per-channel-sender"}"#,
            DmScope::PerChannelSender,
        ),
    ];
    for (json, expected) in &scopes {
        let config: ProcessorConfig = serde_json::from_str(json).unwrap();
        match config {
            ProcessorConfig::SessionRouter { dm_scope } => {
                assert_eq!(dm_scope, *expected, "scope mismatch for {json}");
            }
            _ => panic!("expected SessionRouter variant for {json}"),
        }
    }
}
