use super::*;
use crate::processor_chain::context::MessageContext;
use closeclaw_common::im_plugin::NormalizedMessage;
use tempfile::TempDir;

fn make_ctx(content: &str, channel: &str) -> MessageContext {
    let msg = NormalizedMessage {
        platform: channel.to_string(),
        sender_id: "sender_1".to_string(),
        peer_id: String::new(),
        content: content.to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let mut ctx = MessageContext::from_normalized(msg);
    ctx.metadata
        .insert("channel".to_string(), channel.to_string());
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

    let ctx = make_ctx("hello", "terminal");
    let result = processor.process(&ctx).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_write_file_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(true, tmp.path().to_path_buf(), 7);
    let processor = OutboundRawLogProcessor::new(config);

    let ctx = make_ctx("hi there", "feishu");
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

    let mut ctx = make_ctx("hi there", "feishu");
    ctx.metadata
        .insert("message_id".to_string(), "msg_42".to_string());
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

    let msg = NormalizedMessage {
        platform: "wecom".to_string(),
        sender_id: "s".to_string(),
        peer_id: String::new(),
        content: "hello".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        quoted_message: None,
        thread_id: None,
        account_id: String::new(),
    };
    let inbound_ctx = MessageContext::from_normalized(msg);
    inbound.process(&inbound_ctx).await.unwrap();

    let mut outbound_ctx = make_ctx("reply", "wecom");
    outbound_ctx
        .metadata
        .insert("message_id".to_string(), "msg_99".to_string());
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

    let mut ctx = make_ctx("output text", "terminal");
    ctx.metadata
        .insert("session_key".to_string(), "sess_1".to_string());
    // Set content_blocks since the OutboundRawLogProcessor passes them through
    ctx.content_blocks = vec![closeclaw_llm::types::ContentBlock::Text(
        "output text".to_string(),
    )];

    let result = processor.process(&ctx).await.unwrap().unwrap();
    assert_eq!(result.text_content(), Some("output text"));
    assert_eq!(
        result.metadata.get("session_key").map(|s| s.as_str()),
        Some("sess_1")
    );
    assert_eq!(result.content_blocks.len(), 1);
}
