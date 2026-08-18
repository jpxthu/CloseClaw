//! Streaming error degradation tests (Step 1.3).
//!
//! Verifies that `send_outbound_streaming` correctly propagates
//! `StreamEvent::Error` as `GatewayError::StreamError` with partial
//! content preserved, and that the error includes content blocks
//! received before the error occurred.

use super::*;

// ═══════════════════════════════════════════════════════════════════════════
// StreamEvent::Error propagation
// ═══════════════════════════════════════════════════════════════════════════

/// StreamEvent::Error mid-stream returns GatewayError::StreamError
/// with the error message and any partial content accumulated so far.
#[tokio::test]
async fn test_stream_error_returns_gateway_stream_error() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![Ok::<_, String>(StreamEvent::Error {
        message: "connection lost".to_string(),
    })];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;

    assert!(
        result.is_err(),
        "StreamEvent::Error should propagate as error"
    );
    match result.unwrap_err() {
        crate::GatewayError::StreamError {
            message,
            partial_content,
        } => {
            assert_eq!(message, "connection lost");
            assert!(
                partial_content.is_empty(),
                "no prior content blocks → partial_content should be empty"
            );
        }
        other => panic!("expected StreamError, got {:?}", other),
    }
}

/// StreamEvent::Error after partial text blocks includes them
/// in the partial_content of the returned StreamError.
#[tokio::test]
async fn test_stream_error_preserves_partial_content() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Hello ".to_string(),
            },
        }),
        // Error before BlockEnd — partial text still in renderer.
        Ok(StreamEvent::Error {
            message: "stream interrupted".to_string(),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;

    let err = result.unwrap_err();
    match err {
        crate::GatewayError::StreamError {
            message,
            partial_content,
        } => {
            assert_eq!(message, "stream interrupted");
            // Partial content should include the flushed text block
            // ("Hello ") from the renderer flush + any block-started blocks.
            let has_text = partial_content
                .iter()
                .any(|b| matches!(b, ContentBlock::Text(t) if t.contains("Hello")));
            assert!(
                has_text,
                "partial_content should contain 'Hello' from flushed text, got: {:?}",
                partial_content
            );
        }
        other => panic!("expected StreamError, got {:?}", other),
    }
}

/// StreamEvent::Error after completed text blocks includes those
/// blocks in partial_content.
#[tokio::test]
async fn test_stream_error_after_completed_blocks_preserves_content() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Complete line\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        // Now start a new block and error.
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: ContentDelta::Text {
                text: "Partial".to_string(),
            },
        }),
        Ok(StreamEvent::Error {
            message: "error mid-stream".to_string(),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;

    let err = result.unwrap_err();
    match err {
        crate::GatewayError::StreamError {
            message,
            partial_content,
        } => {
            assert_eq!(message, "error mid-stream");
            // Should have at least the completed "Complete line" block
            // and the flushed "Partial" text.
            let has_complete = partial_content
                .iter()
                .any(|b| matches!(b, ContentBlock::Text(t) if t.contains("Complete line")));
            let has_partial = partial_content
                .iter()
                .any(|b| matches!(b, ContentBlock::Text(t) if t.contains("Partial")));
            assert!(
                has_complete,
                "partial_content should contain completed block, got: {:?}",
                partial_content
            );
            assert!(
                has_partial,
                "partial_content should contain flushed partial text, got: {:?}",
                partial_content
            );
        }
        other => panic!("expected StreamError, got {:?}", other),
    }
}

/// StreamEvent::Error with no prior content → empty partial_content.
#[tokio::test]
async fn test_stream_error_empty_partial_content() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![Ok::<_, String>(StreamEvent::Error {
        message: "immediate failure".to_string(),
    })];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;

    match result.unwrap_err() {
        crate::GatewayError::StreamError {
            partial_content, ..
        } => {
            assert!(
                partial_content.is_empty(),
                "no prior content → partial_content should be empty"
            );
        }
        other => panic!("expected StreamError, got {:?}", other),
    }
}

/// Plugin.send IS called for text blocks that arrive before the error,
/// and those blocks appear in the partial_content of the StreamError.
/// The partial_content is a snapshot of accumulated content, not a
/// replacement for dispatched content.
#[tokio::test]
async fn test_stream_error_text_sent_before_error_appears_in_partial() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Partial text\n".to_string(),
            },
        }),
        Ok(StreamEvent::Error {
            message: "error".to_string(),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await;

    assert!(result.is_err());
    // Text WAS dispatched to plugin.send before the error.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1, "text block should be sent before error");

    // The error's partial_content includes the text that was accumulated.
    match result.unwrap_err() {
        crate::GatewayError::StreamError {
            partial_content, ..
        } => {
            let has_text = partial_content
                .iter()
                .any(|b| matches!(b, ContentBlock::Text(t) if t.contains("Partial text")));
            assert!(
                has_text,
                "partial_content should contain the text block, got: {:?}",
                partial_content
            );
        }
        other => panic!("expected StreamError, got {:?}", other),
    }
}
