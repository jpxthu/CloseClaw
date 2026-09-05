use super::*;
use crate::outbound_raw_log::OutboundRawLogProcessor;
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
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
    };
    let mut ctx = MessageContext::from_normalized(msg);
    ctx.content_blocks
        .push(closeclaw_llm::types::ContentBlock::Text(
            content.to_string(),
        ));
    ctx.metadata
        .insert("channel".to_string(), channel.to_string());
    ctx
}

#[tokio::test]
async fn test_outbound_phase_and_priority() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(false, Some(tmp.path().to_path_buf()));
    let processor = OutboundRawLogProcessor::new(config);
    assert_eq!(processor.phase(), ProcessPhase::Outbound);
    assert_eq!(processor.priority(), 20);
    assert_eq!(processor.name(), "outbound_raw_log");
}

#[tokio::test]
async fn test_passthrough_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(false, Some(tmp.path().to_path_buf()));
    let processor = OutboundRawLogProcessor::new(config);

    let ctx = make_ctx("hello", "terminal");
    let result = processor.process(&ctx).await.unwrap();
    let msg = result.expect("disabled processor should return Some (passthrough)");
    assert_eq!(msg.text_content(), Some("hello"));
    // no log file should be written
    assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());
}

#[tokio::test]
async fn test_write_file_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(true, Some(tmp.path().to_path_buf()));
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
    let config = RawLogConfig::new(true, Some(tmp.path().to_path_buf()));
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
    let config = RawLogConfig::new(true, Some(tmp.path().to_path_buf()));

    let inbound = super::super::raw_log_processor::RawLogProcessor::new(config.clone());
    let outbound = OutboundRawLogProcessor::new(config);

    let msg = NormalizedMessage {
        platform: "wecom".to_string(),
        sender_id: "s".to_string(),
        peer_id: String::new(),
        content: "hello".to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        thread_id: None,
        reply_ref: None,
        account_id: String::new(),
        ..Default::default()
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
async fn test_enabled_but_no_dir_returns_passthrough() {
    let config = RawLogConfig::new(true, None);
    let processor = OutboundRawLogProcessor::new(config);

    let ctx = make_ctx("hello", "terminal");
    let result = processor.process(&ctx).await.unwrap();
    let msg = result.expect("enabled with no dir should passthrough (not None)");
    assert_eq!(msg.text_content(), Some("hello"));
}

#[tokio::test]
async fn test_preserves_content_and_blocks() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(true, Some(tmp.path().to_path_buf()));
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

#[tokio::test]
async fn test_error_when_dir_does_not_exist() {
    let config = RawLogConfig::new(true, Some("/nonexistent/path".into()));
    let processor = OutboundRawLogProcessor::new(config);

    let ctx = make_ctx("hello", "terminal");
    let err = processor.process(&ctx).await.unwrap_err();
    assert!(
        matches!(err, ProcessError::ProcessorFailed { .. }),
        "write failure on missing dir should yield ProcessorFailed, got: {err:?}"
    );
}

#[tokio::test]
async fn test_disabled_to_enabled_transition() {
    // Start disabled — passthrough, no log
    let tmp = TempDir::new().unwrap();
    let config_off = RawLogConfig::new(false, Some(tmp.path().to_path_buf()));
    let processor_off = OutboundRawLogProcessor::new(config_off);

    let ctx1 = make_ctx("before", "feishu");
    let result1 = processor_off.process(&ctx1).await.unwrap();
    let msg1 = result1.unwrap();
    assert_eq!(msg1.text_content(), Some("before"));
    assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());

    // Switch to enabled — should write log
    let config_on = RawLogConfig::new(true, Some(tmp.path().to_path_buf()));
    let processor_on = OutboundRawLogProcessor::new(config_on);

    let ctx2 = make_ctx("after", "feishu");
    let result2 = processor_on.process(&ctx2).await.unwrap();
    let msg2 = result2.unwrap();
    assert_eq!(msg2.text_content(), Some("after"));
    let files: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
    assert_eq!(files.len(), 1, "should have one log file after enabling");
}

#[tokio::test]
async fn test_disabled_with_dir_present_does_not_write() {
    let tmp = TempDir::new().unwrap();
    let config = RawLogConfig::new(false, Some(tmp.path().to_path_buf()));
    let processor = OutboundRawLogProcessor::new(config);

    let ctx = make_ctx("test", "terminal");
    let result = processor.process(&ctx).await.unwrap();
    let msg = result.unwrap();
    assert_eq!(msg.text_content(), Some("test"));
    // disabled + dir present → passthrough, no file written
    assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());
}
