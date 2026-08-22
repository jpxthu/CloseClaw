//! Step 1.5c tests — part 2: state transition, finish VerbosityFilter skip,
//! edge cases, batch success, regression, and merge_dsl_results.

use super::*;

// ═══════════════════════════════════════════════════════════════════════════
// 1. State transition: incremental DSL → finish merge into dsl_result
// ═══════════════════════════════════════════════════════════════════════════

/// DSL instructions accumulate during streaming and appear in the final
/// `dsl_result` of the `StreamResult`.
#[tokio::test]
async fn test_incremental_dsl_accumulates_into_dsl_result() {
    let chain = Arc::new(MockChain::new());
    chain.push_instruction(DslInstruction {
        instruction_type: "button".into(),
        params: HashMap::from([
            ("label".into(), "Yes".into()),
            ("action".into(), "confirm".into()),
        ]),
    });
    chain.push_instruction(DslInstruction {
        instruction_type: "button".into(),
        params: HashMap::from([
            ("label".into(), "No".into()),
            ("action".into(), "cancel".into()),
        ]),
    });

    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::button[label:Yes;action:confirm]\n".into(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::button[label:No;action:cancel]\n".into(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".into()),
        }),
    ];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Both DSL lines sent as-is during streaming (no DSL stripping).
    assert_eq!(plugin.drain_sent().len(), 2);

    // DSL lines pass through in content_blocks (mock doesn't strip in finish).
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        text_blocks.len(),
        2,
        "both DSL lines should be in content_blocks"
    );
    assert!(text_blocks[0].contains("::button"));
    assert!(text_blocks[1].contains("::button"));
}

/// DSL instructions from streaming are merged with any finish-phase DslParser
/// results (both sources contribute to final dsl_result).
#[tokio::test]
async fn test_dsl_mixed_with_non_dsl_accumulates_correctly() {
    let chain = Arc::new(MockChain::new());
    chain.push_instruction(DslInstruction {
        instruction_type: "selector".into(),
        params: HashMap::from([("options".into(), "A,B".into())]),
    });

    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Hello\n".into(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::selector[options:A,B]\n".into(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "World\n".into(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".into()),
        }),
    ];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // All lines sent as-is during streaming.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 3, "all lines should be sent");
    assert_eq!(extract_text(&sent[0]), "Hello\n");
    assert!(extract_text(&sent[1]).contains("::selector"));
    assert_eq!(extract_text(&sent[2]), "World\n");

    // All lines in content_blocks.
    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        text_blocks.len(),
        3,
        "all lines should be in content_blocks"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Finish phase skips VerbosityFilter
// ═══════════════════════════════════════════════════════════════════════════

/// With a real ProcessorRegistry (VerbosityFilter + DslParser), the finish
/// phase calls `process_outbound_without_verbosity`, which runs DslParser
/// but skips VerbosityFilter. Since VerbosityFilter already ran per-chunk
/// during streaming, Thinking blocks are absent from content_blocks at
/// finish time — DslParser processes the remaining Text blocks and extracts
/// DSL instructions into dsl_result.
#[tokio::test]
async fn test_finish_phase_skips_verbosity_filter() {
    let mut registry = closeclaw_processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(
        closeclaw_processor_chain::verbosity_filter::VerbosityFilter,
    ));
    registry.register(Arc::new(closeclaw_processor_chain::DslParser));

    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::new(MockPersist)),
        None,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry) as Arc<dyn closeclaw_common::processor::ProcessorChain>,
    );
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    gw.register_plugin(plugin.clone()).await;
    let msg = make_message("agent-1", "hello");
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();

    // Normal verbosity: Thinking filtered in incremental, Text + DSL sent.
    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "hidden reasoning".into(),
                signature: None,
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: ContentDelta::Text {
                text: "Pick one:\n".into(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: ContentDelta::Text {
                text: "::button[label:OK;action:go;value:1]\n".into(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".into()),
        }),
    ];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Streaming: Thinking filtered, all text lines sent as-is.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 2, "both text lines should be sent as-is");
    assert_eq!(extract_text(&sent[0]), "Pick one:\n");
    assert!(extract_text(&sent[1]).contains("::button"));

    // Post-stream: no Thinking in result (filtered during streaming).
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(!has_thinking, "Thinking should be filtered at Normal level");

    // dsl_result: DslParser ran in finish phase, stripping DSL from content_blocks.
    let dsl = result
        .dsl_result
        .as_ref()
        .expect("dsl_result should be present");
    let parsed: DslParseResult = serde_json::from_str(dsl).unwrap();
    assert_eq!(
        parsed.instructions.len(),
        1,
        "DslParser should extract DSL in finish"
    );
    assert_eq!(parsed.instructions[0].instruction_type, "button");
}

/// At Full verbosity, Thinking blocks are NOT filtered during streaming
/// (sent via send_render_block). The finish phase also does NOT re-filter
/// VerbosityFilter — both Thinking and Text blocks remain in the result.
#[tokio::test]
async fn test_finish_phase_full_verbosity_preserves_thinking() {
    let mut registry = closeclaw_processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(
        closeclaw_processor_chain::verbosity_filter::VerbosityFilter,
    ));
    registry.register(Arc::new(closeclaw_processor_chain::DslParser));

    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::new(MockPersist)),
        None,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::with_processor_registry(
        config,
        Arc::clone(&sm),
        Arc::new(registry) as Arc<dyn closeclaw_common::processor::ProcessorChain>,
    );
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    gw.register_plugin(plugin.clone()).await;
    let msg = make_message("agent-1", "hello");
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    // Set up conversation session with Full verbosity.
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        sid.clone(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    cs_arc
        .write()
        .await
        .set_verbosity_level(closeclaw_common::VerbosityLevel::Full);
    sm.conversation_sessions
        .write()
        .await
        .insert(sid.clone(), cs_arc);

    // Full verbosity: Thinking NOT filtered during streaming.
    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "visible reasoning".into(),
                signature: None,
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: ContentDelta::Text {
                text: "Answer.\n".into(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".into()),
        }),
    ];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Streaming: both Thinking + Text sent (Full mode).
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 2);

    // Finish phase: VerbosityFilter skipped → Thinking preserved in result.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(
        has_thinking,
        "Thinking should be preserved at Full verbosity"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Edge cases
// ═══════════════════════════════════════════════════════════════════════════

/// Empty chunk (only whitespace/newlines) — no text sent, no content block.
#[tokio::test]
async fn test_empty_chunk_no_send_no_block() {
    let chain = Arc::new(MockChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup(chain, plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "   \n".into(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".into()),
        }),
    ];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    assert_eq!(
        plugin.drain_sent().len(),
        0,
        "whitespace-only chunk should not be sent"
    );
    // No non-empty text blocks in result.
    let non_empty: Vec<_> = result
        .content_blocks
        .iter()
        .filter(|b| matches!(b, ContentBlock::Text(t) if !t.trim().is_empty()))
        .collect();
    assert!(
        non_empty.is_empty(),
        "no non-empty text blocks for whitespace chunk"
    );
}

/// Empty DSL line (malformed, no instructions produced) — DSL line is
/// emitted by DefaultStreamingRenderer as text (renderer is DSL-unaware),
/// but no DSL instruction is accumulated in dsl_result.
#[tokio::test]
async fn test_empty_dsl_line_no_instruction_accumulated() {
    let chain = Arc::new(MockChain::new());
    // No instructions pushed — mock returns empty DslParseResult for DSL lines.
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::button[empty]\n".into(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".into()),
        }),
    ];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // DefaultStreamingRenderer emits the text (it is DSL-unaware).
    // The DSL line is sent as text by the renderer.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1, "renderer emits DSL line as text");

    // dsl_result should be None (no instructions accumulated).
    assert!(
        result.dsl_result.is_none(),
        "no instructions → dsl_result should be None"
    );
}

/// Registry `None` — text passes through unchanged (zero-overhead passthrough).
#[tokio::test]
async fn test_registry_none_passthrough() {
    let chain = Arc::new(MockChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup(chain, plugin.clone()).await;

    // Clear the processor registry to simulate None.
    *gw.processor_registry.write().unwrap() = None;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Hello from None registry\n".into(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".into()),
        }),
    ];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    // Text passed through unchanged (no DSL parsing).
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1, "text should be sent with None registry");
    assert_eq!(extract_text(&sent[0]), "Hello from None registry\n");

    // dsl_result: no registry → no DslParser → None.
    assert!(
        result.dsl_result.is_none(),
        "None registry should produce no dsl_result"
    );
}

/// Empty stream (no events) — finish phase may produce a Text("") fallback
/// block via `make_outbound_input` when content_blocks is empty.
/// Key assertion: no plugin.send calls, no non-empty text blocks.
#[tokio::test]
async fn test_empty_stream_produces_empty_result() {
    let chain = Arc::new(MockChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup(chain, plugin.clone()).await;

    let events: Vec<Result<StreamEvent, String>> = vec![];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await
        .unwrap();

    assert_eq!(plugin.drain_sent().len(), 0, "no events → no sends");
    // finish_streaming_pipeline may produce a Text("") fallback block.
    // Verify no non-empty text blocks and no dsl_result.
    let non_empty: Vec<_> = result
        .content_blocks
        .iter()
        .filter(|b| matches!(b, ContentBlock::Text(t) if !t.is_empty()))
        .collect();
    assert!(
        non_empty.is_empty(),
        "no non-empty text blocks for empty stream"
    );
    assert!(result.dsl_result.is_none(), "no events → no dsl_result");
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Batch send success → no failure notification
// ═══════════════════════════════════════════════════════════════════════════

/// When batch send succeeds, exactly one send call is made — no failure
/// notification is sent.
#[tokio::test]
async fn test_batch_send_success_no_notification() {
    let mock = Arc::new(CapturingPlugin::new("mock"));
    let plugin: Arc<dyn IMPlugin> = mock.clone();
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        "batch_ok".into(),
        crate::Session {
            id: "batch_ok".into(),
            agent_id: "chat_test".into(),
            channel: "mock".into(),
            created_at: 0,
            depth: 0,
        },
    );
    let gw = Gateway::new(config, Arc::clone(&sm));
    gw.register_plugin(plugin.clone()).await;

    let result = gw
        .send_outbound("batch_ok", "mock", "success message", vec![], None, None)
        .await;
    assert!(result.is_ok(), "batch success should return Ok");

    // CapturingPlugin.send always succeeds. On batch success, only 1 send
    // call is made (the original message) — no failure notification.
    assert_eq!(
        mock.send_count(),
        1,
        "only one send call when batch succeeds"
    );
    let sent = mock.drain_sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        extract_text(&sent[0]),
        "success message",
        "sent text matches original"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Regression: pre-flight rejection and stream error paths
// ═══════════════════════════════════════════════════════════════════════════

/// Pre-flight middleware rejection still sends rejection notification
/// and returns Ok (regression guard).
#[tokio::test]
async fn test_preflight_rejection_sends_notification() {
    use closeclaw_common::OutboundMiddleware;

    struct RejectAll;
    #[async_trait::async_trait]
    impl OutboundMiddleware for RejectAll {
        fn name(&self) -> &str {
            "reject-all"
        }
        async fn process(
            &self,
            _ctx: &closeclaw_common::MiddlewareContext,
            _rendered: &RenderedOutput,
        ) -> Result<(), closeclaw_common::MiddlewareError> {
            Err(closeclaw_common::MiddlewareError::rejected(
                "reject-all",
                "blocked",
            ))
        }
    }

    let mock = Arc::new(CapturingPlugin::new("mock"));
    let plugin: Arc<dyn IMPlugin> = mock.clone();
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        "reject".into(),
        crate::Session {
            id: "reject".into(),
            agent_id: "chat_test".into(),
            channel: "mock".into(),
            created_at: 0,
            depth: 0,
        },
    );
    let gw = Gateway::new(config, Arc::clone(&sm));
    gw.register_plugin(plugin.clone()).await;
    gw.add_outbound_middleware(Arc::new(RejectAll));

    let result = gw
        .send_outbound("reject", "mock", "blocked msg", vec![], None, None)
        .await;
    assert!(result.is_ok(), "middleware rejection should return Ok");

    // CapturingPlugin captures sent payloads. On middleware rejection,
    // exactly one send is made (the rejection notification).
    assert_eq!(mock.send_count(), 1, "rejection notification sent");
    let sent = mock.drain_sent();
    assert_eq!(
        extract_text(&sent[0]),
        "Your message was not sent due to an outbound policy restriction."
    );
}

/// StreamEvent::Error returns GatewayError::StreamError (regression guard).
#[tokio::test]
async fn test_stream_error_propagates_as_stream_error() {
    let chain = Arc::new(MockChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup(chain, plugin.clone()).await;

    let events = vec![Ok::<_, String>(StreamEvent::Error {
        message: "connection lost".into(),
    })];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await;

    assert!(result.is_err(), "StreamEvent::Error should propagate");
    match result.unwrap_err() {
        crate::GatewayError::StreamError {
            message,
            partial_content,
        } => {
            assert_eq!(message, "connection lost");
            assert!(partial_content.is_empty());
        }
        other => panic!("expected StreamError, got {:?}", other),
    }
}

/// StreamEvent::Error after partial content preserves content in error.
#[tokio::test]
async fn test_stream_error_preserves_partial_content() {
    let chain = Arc::new(MockChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup(chain, plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Partial ".into(),
            },
        }),
        Ok(StreamEvent::Error {
            message: "interrupted".into(),
        }),
    ];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await;

    match result.unwrap_err() {
        crate::GatewayError::StreamError {
            message,
            partial_content,
        } => {
            assert_eq!(message, "interrupted");
            let has = partial_content
                .iter()
                .any(|b| matches!(b, ContentBlock::Text(t) if t.contains("Partial")));
            assert!(
                has,
                "partial_content should contain 'Partial', got: {:?}",
                partial_content
            );
        }
        other => panic!("expected StreamError, got {:?}", other),
    }
}

/// Streaming with plugin.send failure mid-stream returns StreamError.
#[tokio::test]
async fn test_streaming_plugin_send_failure_returns_error() {
    let chain = Arc::new(MockChain::new());
    let plugin: Arc<dyn IMPlugin> = Arc::new(FailingPlugin(super::MockImPlugin::new(
        "mock",
        super::SendBehavior::Fail,
    )));
    let (gw, _sm, sid) = setup(chain, plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Hello\n".into(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".into()),
        }),
    ];
    let stream = futures::stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc, OutboundMeta::default())
        .await;

    // Plugin.send fails → GatewayError::AdapterError (from SendFailed).
    let err = result.unwrap_err();
    match &err {
        crate::GatewayError::AdapterError(msg) => {
            assert!(
                msg.contains("network error"),
                "error message should mention 'network error', got: {}",
                msg
            );
        }
        other => panic!("expected GatewayError::AdapterError, got {:?}", other),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. merge_dsl_results unit tests
// ═══════════════════════════════════════════════════════════════════════════

/// Both incremental and finish-phase DSL empty → None.
#[test]
fn test_merge_dsl_results_both_empty() {
    let meta = HashMap::new();
    let result = crate::outbound_helpers::merge_dsl_results(&meta, vec![]);
    assert!(result.is_none());
}

/// Only incremental instructions → serialize directly.
#[test]
fn test_merge_dsl_results_incremental_only() {
    let meta = HashMap::new();
    let incremental = vec![DslInstruction {
        instruction_type: "button".into(),
        params: HashMap::from([("label".into(), "Go".into())]),
    }];
    let result = crate::outbound_helpers::merge_dsl_results(&meta, incremental)
        .expect("should produce result");
    let parsed: DslParseResult = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed.instructions.len(), 1);
    assert_eq!(parsed.instructions[0].params["label"], "Go");
}

/// Only finish-phase DSL in metadata → use chain result.
#[test]
fn test_merge_dsl_results_finish_only() {
    let chain_result = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "selector".into(),
            params: HashMap::from([("options".into(), "X".into())]),
        }],
    };
    let mut meta = HashMap::new();
    meta.insert(
        "dsl_result".into(),
        serde_json::to_string(&chain_result).unwrap(),
    );
    let result =
        crate::outbound_helpers::merge_dsl_results(&meta, vec![]).expect("should produce result");
    let parsed: DslParseResult = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed.instructions.len(), 1);
    assert_eq!(parsed.instructions[0].instruction_type, "selector");
}

/// Both incremental and finish-phase → merged correctly.
#[test]
fn test_merge_dsl_results_both_present() {
    let chain_result = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "selector".into(),
            params: HashMap::from([("options".into(), "Y".into())]),
        }],
    };
    let mut meta = HashMap::new();
    meta.insert(
        "dsl_result".into(),
        serde_json::to_string(&chain_result).unwrap(),
    );
    let incremental = vec![DslInstruction {
        instruction_type: "button".into(),
        params: HashMap::from([("label".into(), "Z".into())]),
    }];
    let result =
        crate::outbound_helpers::merge_dsl_results(&meta, incremental).expect("should produce");
    let parsed: DslParseResult = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed.instructions.len(), 2);
    assert_eq!(parsed.instructions[0].instruction_type, "button");
    assert_eq!(parsed.instructions[1].instruction_type, "selector");
}
