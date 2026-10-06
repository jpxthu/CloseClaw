//! Behavior tests for the composition-root-injected outbound raw-log seam.
//!
//! The simplified outbound path writes one raw-log snapshot through the
//! gateway-local [`OutboundRawLogWriter`] abstraction. These tests pin the
//! two injection branches: an injected writer receives exactly one complete
//! snapshot; without an injection the write is skipped and the message is
//! forwarded unchanged. (The write-failure branch is covered by
//! `outbound_tests::test_process_outbound_raw_log_only_fail_open`.)

use std::sync::Arc;

use closeclaw_llm::types::ContentBlock;
use closeclaw_session::persistence::ReasoningLevel;

use crate::outbound_raw_log::OutboundRawLogSnapshot;
use crate::{Gateway, GatewayConfig, SessionManager};

#[tokio::test]
async fn test_process_outbound_raw_log_only_uses_injected_writer() {
    // The composition-root-injected raw-log writer must receive exactly one
    // snapshot carrying the raw content, the message blocks and the channel.
    struct RecordingRawLogWriter {
        snapshots: std::sync::Mutex<Vec<OutboundRawLogSnapshot>>,
    }

    #[async_trait::async_trait]
    impl crate::outbound_raw_log::OutboundRawLogWriter for RecordingRawLogWriter {
        async fn write(&self, snapshot: OutboundRawLogSnapshot) -> Result<(), String> {
            self.snapshots.lock().expect("lock").push(snapshot);
            Ok(())
        }
    }

    let config = GatewayConfig {
        name: "test-rawlog-injected".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let writer = Arc::new(RecordingRawLogWriter {
        snapshots: std::sync::Mutex::new(Vec::new()),
    });
    let gw = Gateway::new(
        config,
        sm,
        Arc::new(closeclaw_processor_chain::ProcessorRegistry::new()),
        Some(writer.clone()),
    );

    let blocks = vec![ContentBlock::Text("simplified output".into())];
    let result = gw
        .process_outbound_raw_log_only("simplified output", blocks.clone(), "feishu")
        .await;
    assert!(
        result.is_ok(),
        "injected writer succeeded, got {:?}",
        result.err()
    );
    let snapshots = writer.snapshots.lock().expect("lock");
    assert_eq!(
        snapshots.len(),
        1,
        "exactly one raw-log snapshot must be written"
    );
    let snapshot = &snapshots[0];
    assert_eq!(snapshot.content, "simplified output");
    assert_eq!(snapshot.content_blocks, blocks);
    assert_eq!(
        snapshot.metadata.get("channel").map(String::as_str),
        Some("feishu"),
        "the channel must reach the injected writer through the snapshot metadata"
    );
}

#[tokio::test]
async fn test_process_outbound_raw_log_only_without_writer_skips_write() {
    // No composition-root injection (bypass / test construction): the write
    // is skipped and the message is forwarded unchanged.
    let config = GatewayConfig {
        name: "test-rawlog-no-writer".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::new_for_tests(config, sm);
    assert!(
        gw.outbound_raw_log.is_none(),
        "the non-injection branch must build the gateway without a raw-log writer"
    );

    let blocks = vec![ContentBlock::Text("forwarded anyway".into())];
    let result = gw
        .process_outbound_raw_log_only("forwarded anyway", blocks.clone(), "mock")
        .await;
    assert!(
        result.is_ok(),
        "a missing writer must not fail the simplified path, got {:?}",
        result.err()
    );
    let msg = result.unwrap();
    assert_eq!(
        msg.content_blocks, blocks,
        "the message must be forwarded unchanged when no writer is injected"
    );
}
