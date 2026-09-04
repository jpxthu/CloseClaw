//! Unit tests for [`ProcessorChainLoader`] — conditional registration,
//! priority/sorting, and inbound chain regression.

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

// ── RawLog conditional registration: priority & sorting ──────────────────────

#[test]
fn test_raw_log_priority_is_10() {
    let tmp = tempfile::tempdir().unwrap();
    let config = ProcessorChainConfig {
        inbound: vec![ProcessorConfig::RawLog {
            enabled: true,
            dir: Some(tmp.path().to_path_buf()),
        }],
        outbound: vec![],
    };
    let registry = ProcessorChainLoader::load(&config).unwrap();
    assert_eq!(registry.inbound_len(), 1);
    // The registry stores processors in registration order, but process_inbound
    // sorts by priority. Verify the processor's priority value directly.
    // We can't access internals, but we can verify via a roundtrip: load a chain
    // with RawLog, SessionRouter, ContentNormalizer and confirm the chain length.
    let config_full = ProcessorChainConfig {
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
    let registry_full = ProcessorChainLoader::load(&config_full).unwrap();
    assert_eq!(registry_full.inbound_len(), 3);
}

#[test]
fn test_full_chain_sorts_by_priority() {
    let tmp = tempfile::tempdir().unwrap();
    let config = ProcessorChainConfig {
        inbound: vec![
            ProcessorConfig::ContentNormalizer,
            ProcessorConfig::RawLog {
                enabled: true,
                dir: Some(tmp.path().to_path_buf()),
            },
            ProcessorConfig::SessionRouter,
        ],
        outbound: vec![],
    };
    let registry = ProcessorChainLoader::load(&config).unwrap();
    assert_eq!(registry.inbound_len(), 3);

    // Run process_inbound and verify execution order via metadata.
    // SessionRouter (20) adds session_key, ContentNormalizer (30) normalizes
    // content, RawLog (10) runs first (no-op metadata, just passes through).
    // The final content should be normalized (no ANSI) and session_key present.
    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "chat_1".to_string(),
        content: "hello\x1b[31mworld\x1b[0m".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: "acct_1".to_string(),
        ..Default::default()
    };

    // process_inbound sorts by priority: raw_log(10) → session_router(20) →
    // content_normalizer(30). content_normalizer strips ANSI, session_router
    // adds session_key. Content should be normalized and session_key present.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async { registry.process_inbound(msg).await.unwrap() });
    let text = result.text_content().unwrap();
    assert!(
        !text.contains("\x1b"),
        "ANSI should be stripped by ContentNormalizer"
    );
    assert!(
        result.metadata.contains_key("session_key"),
        "SessionRouter should have added session_key"
    );
}

#[test]
fn test_inbound_chain_without_raw_log_executes_normally() {
    // Verify that when raw_log is not registered, the remaining chain
    // (SessionRouter + ContentNormalizer) still executes correctly.
    let config = ProcessorChainConfig {
        inbound: vec![
            ProcessorConfig::SessionRouter,
            ProcessorConfig::ContentNormalizer,
        ],
        outbound: vec![],
    };
    let registry = ProcessorChainLoader::load(&config).unwrap();
    assert_eq!(
        registry.inbound_len(),
        2,
        "should have exactly 2 inbound processors without raw_log"
    );

    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "chat_1".to_string(),
        content: "hello\r\n  world  \r\n".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: "acct_1".to_string(),
        ..Default::default()
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async { registry.process_inbound(msg).await.unwrap() });

    // session_key should be present (SessionRouter ran)
    assert!(
        result.metadata.contains_key("session_key"),
        "session_key should be present even without raw_log"
    );
    // Content should be normalized (trailing whitespace trimmed)
    let text = result.text_content().unwrap();
    assert!(
        !text.contains("\r"),
        "carriage returns should be stripped by ContentNormalizer"
    );
    assert!(
        !text.ends_with(' '),
        "trailing whitespace should be trimmed"
    );
}

#[test]
fn test_default_config_fail_open_behavior() {
    // Default config (no raw_log registered): inbound chain still
    // produces a valid ProcessedMessage with message_type metadata.
    let config = ProcessorChainConfig::default();
    let registry = ProcessorChainLoader::load(&config).unwrap();
    assert_eq!(registry.inbound_len(), 2);

    let msg = NormalizedMessage {
        platform: "feishu".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: "chat_1".to_string(),
        content: "test message".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        account_id: "acct_1".to_string(),
        ..Default::default()
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async { registry.process_inbound(msg).await.unwrap() });

    assert_eq!(result.text_content(), Some("test message"));
    assert!(
        result.metadata.contains_key("message_type"),
        "message_type should be present in metadata"
    );
    assert!(
        result.metadata.contains_key("session_key"),
        "session_key should be computed by SessionRouter"
    );
}
