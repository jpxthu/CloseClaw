use super::*;
use crate::processor_chain::context::{MessageContext, RawMessage};
use chrono::Utc;
use tempfile::TempDir;

fn make_ctx(content: &str, channel: &str, message_id: &str) -> MessageContext {
    let raw = RawMessage {
        platform: channel.to_string(),
        sender_id: "sender_1".to_string(),
        peer_id: String::new(),
        content: content.to_string(),
        timestamp: Utc::now(),
        message_id: message_id.to_string(),
        account_id: None,
    };
    let mut ctx = MessageContext::from_raw(raw);
    ctx.metadata.insert(
        "channel".to_string(),
        serde_json::Value::String(channel.to_string()),
    );
    ctx
}

#[tokio::test]
async fn test_outbound_phase_and_priority() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(false, tmp.path().to_path_buf(), 7);
    let processor = OutboundRawLogProcessor::new(config);
    assert_eq!(processor.phase(), ProcessPhase::Outbound);
    assert_eq!(processor.priority(), 20);
    assert_eq!(processor.name(), "outbound_raw_log");
}

#[tokio::test]
async fn test_bypass_when_disabled_and_no_debug() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(false, tmp.path().to_path_buf(), 7);
    let processor = OutboundRawLogProcessor::new(config);

    let ctx = make_ctx("hello", "terminal", "msg_1");
    let result = processor.process(&ctx).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_write_file_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);
    let processor = OutboundRawLogProcessor::new(config);

    let ctx = make_ctx("hi there", "feishu", "msg_42");
    let result = processor.process(&ctx).await.unwrap();
    assert!(result.is_some());

    let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);

    let name = files[0].file_name();
    let name_str = name.to_string_lossy();
    assert!(
        name_str.contains("_outbound_"),
        "filename should contain _outbound_: {name_str}"
    );
    assert!(
        name_str.starts_with("feishu_outbound_"),
        "filename: {name_str}"
    );
    assert!(
        name_str.ends_with(".json"),
        "filename should end with .json: {name_str}"
    );
}

#[tokio::test]
async fn test_write_file_with_message_id_metadata() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);
    let processor = OutboundRawLogProcessor::new(config);

    let mut ctx = make_ctx("hi there", "feishu", "msg_42");
    ctx.metadata.insert(
        "message_id".to_string(),
        serde_json::Value::String("msg_42".to_string()),
    );
    let result = processor.process(&ctx).await.unwrap();
    assert!(result.is_some());

    let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);

    let name = files[0].file_name();
    let name_str = name.to_string_lossy();
    assert!(
        name_str.starts_with("feishu_outbound_"),
        "filename: {name_str}"
    );
    assert!(name_str.ends_with("_msg_42.json"), "filename: {name_str}");
}

#[tokio::test]
async fn test_outbound_and_independent_from_inbound() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);

    let inbound = super::super::raw_log_processor::RawLogProcessor::new(config.clone()).unwrap();
    let outbound = OutboundRawLogProcessor::new(config);

    let raw = RawMessage {
        platform: "wecom".to_string(),
        sender_id: "s".to_string(),
        peer_id: String::new(),
        content: "hello".to_string(),
        timestamp: Utc::now(),
        message_id: "msg_99".to_string(),
        account_id: None,
    };
    let inbound_ctx = MessageContext::from_raw(raw.clone());
    inbound.process(&inbound_ctx).await.unwrap();

    let mut outbound_ctx = make_ctx("reply", "wecom", "msg_99");
    outbound_ctx.metadata.insert(
        "message_id".to_string(),
        serde_json::Value::String("msg_99".to_string()),
    );
    outbound.process(&outbound_ctx).await.unwrap();

    let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
    assert_eq!(files.len(), 2);

    let names: Vec<_> = files
        .iter()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().any(|n| !n.contains("_outbound_")),
        "should have an inbound log: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.contains("_outbound_")),
        "should have an outbound log: {names:?}"
    );
}

#[tokio::test]
async fn test_preserves_content_and_blocks() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);
    let processor = OutboundRawLogProcessor::new(config);

    let mut ctx = make_ctx("output text", "terminal", "msg_5");
    ctx.metadata.insert(
        "session_key".to_string(),
        serde_json::Value::String("sess_1".to_string()),
    );

    let result = processor.process(&ctx).await.unwrap().unwrap();
    assert_eq!(result.content, "output text");
    assert_eq!(
        result.metadata.get("session_key").and_then(|v| v.as_str()),
        Some("sess_1")
    );
    assert!(result.content_blocks.is_empty());
}
