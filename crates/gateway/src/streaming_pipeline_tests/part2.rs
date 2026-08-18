//! Additional streaming pipeline tests (split from streaming_pipeline_tests.rs
//! to comply with the 1000-line file limit).
//!
//! Covers:
//! - State transition: DslParseResult accumulates correctly, merges post-stream
//! - Block flushing: partial text at BlockEnd is dispatched
//! - MessageEnd flushing: remaining text dispatched at MessageEnd
//! - Verbosity filtering interaction with streaming pipeline

use super::*;

// ═══════════════════════════════════════════════════════════════════════════
// State transition: DslParseResult accumulates and merges post-stream
// ═══════════════════════════════════════════════════════════════════════════

/// All lines are sent as-is during streaming — no DSL parsing in incremental
/// phase. DSL is deferred to post-stream Processor Chain.
#[tokio::test]
async fn test_streaming_dsl_results_accumulate_and_merge() {
    let chain = Arc::new(MockProcessorChain::new());
    chain.push_dsl_instruction(closeclaw_common::processor::DslInstruction {
        instruction_type: "button".to_string(),
        params: HashMap::from([
            ("label".to_string(), "Yes".to_string()),
            ("action".to_string(), "confirm".to_string()),
            ("value".to_string(), "1".to_string()),
        ]),
    });
    chain.push_dsl_instruction(closeclaw_common::processor::DslInstruction {
        instruction_type: "button".to_string(),
        params: HashMap::from([
            ("label".to_string(), "No".to_string()),
            ("action".to_string(), "cancel".to_string()),
            ("value".to_string(), "0".to_string()),
        ]),
    });

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
                text: "Please choose:\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::button[label:Yes;action:confirm;value:1]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::button[label:No;action:cancel;value:0]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let _result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // parse_line_for_dsl should NOT be called during streaming (DSL deferred).
    let parsed = chain.parsed_lines();
    assert!(
        parsed.is_empty(),
        "parse_line_for_dsl should not be called during streaming"
    );

    let sent = plugin.drain_sent();
    // All lines sent as-is (no DSL stripping).
    assert_eq!(sent.len(), 3, "all lines should be sent");
    assert_eq!(extract_text(&sent[0]), "Please choose:\n");
    assert_eq!(
        extract_text(&sent[1]),
        "::button[label:Yes;action:confirm;value:1]\n"
    );
    assert_eq!(
        extract_text(&sent[2]),
        "::button[label:No;action:cancel;value:0]\n"
    );
}

/// DSL-only lines are sent as-is during streaming (no DSL stripping).
#[tokio::test]
async fn test_streaming_all_dsl_no_plain_text() {
    let chain = Arc::new(MockProcessorChain::new());
    chain.push_dsl_instruction(closeclaw_common::processor::DslInstruction {
        instruction_type: "button".to_string(),
        params: HashMap::from([
            ("label".to_string(), "OK".to_string()),
            ("action".to_string(), "submit".to_string()),
            ("value".to_string(), "yes".to_string()),
        ]),
    });
    chain.push_dsl_instruction(closeclaw_common::processor::DslInstruction {
        instruction_type: "selector".to_string(),
        params: HashMap::from([
            ("label".to_string(), "Pick".to_string()),
            ("action".to_string(), "choose".to_string()),
            ("options".to_string(), "A,B".to_string()),
        ]),
    });

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
                text: "::button[label:OK;action:submit;value:yes]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::selector[label:Pick;action:choose;options:A,B]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // parse_line_for_dsl should NOT be called during streaming (DSL deferred).
    let parsed = chain.parsed_lines();
    assert!(
        parsed.is_empty(),
        "parse_line_for_dsl should not be called during streaming"
    );

    // DSL lines are sent as-is during streaming.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 2, "DSL lines should be sent as text");

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
        "DSL lines should be in content_blocks"
    );
}

/// After streaming completes, the post-stream `process_or_bypass`
/// runs the full Processor Chain on content_blocks. All lines
/// (including DSL) are sent as-is during streaming.
#[tokio::test]
async fn test_streaming_dsl_results_not_lost_after_merge() {
    let chain = Arc::new(MockProcessorChain::new());
    chain.push_dsl_instruction(closeclaw_common::processor::DslInstruction {
        instruction_type: "button".to_string(),
        params: HashMap::from([
            ("label".to_string(), "Submit".to_string()),
            ("action".to_string(), "go".to_string()),
            ("value".to_string(), "confirm".to_string()),
        ]),
    });

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
                text: "Choose an option:\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::button[label:Submit;action:go;value:confirm]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    let sent = plugin.drain_sent();
    // All lines sent as-is during streaming (no DSL stripping).
    assert_eq!(sent.len(), 2, "all lines should be sent");
    assert_eq!(extract_text(&sent[0]), "Choose an option:\n");
    assert_eq!(
        extract_text(&sent[1]),
        "::button[label:Submit;action:go;value:confirm]\n"
    );

    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    // Both lines in content_blocks (DSL included).
    assert_eq!(text_blocks.len(), 2);
    assert!(
        text_blocks.contains(&"Choose an option:\n".to_string()),
        "should contain plain text"
    );
    assert!(
        text_blocks.iter().any(|t| t.contains("::button")),
        "should contain DSL line (no stripping in streaming)"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Block flushing: partial text at BlockEnd is dispatched
// ═══════════════════════════════════════════════════════════════════════════

/// When BlockEnd arrives with partial (un-flushed) text in the
/// LineBuffer, the remaining text is dispatched via `dispatch_text`.
#[tokio::test]
async fn test_streaming_block_end_flushes_partial_text() {
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
                text: "partial text".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(extract_text(&sent[0]), "partial text");

    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks, vec!["partial text"]);
}

/// MessageEnd flush also dispatches any remaining text.
#[tokio::test]
async fn test_streaming_message_end_flushes_remaining_text() {
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
                text: "remaining".to_string(),
            },
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(extract_text(&sent[0]), "remaining");

    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks, vec!["remaining"]);
}

// ═══════════════════════════════════════════════════════════════════════════
// Verbosity filtering interaction with streaming pipeline
// ═══════════════════════════════════════════════════════════════════════════

/// Build a real outbound processor chain with VerbosityFilter + DslParser.
/// Used to verify the post-stream pipeline correctly filters blocks.
fn build_outbound_chain() -> closeclaw_processor_chain::ProcessorRegistry {
    let mut registry = closeclaw_processor_chain::ProcessorRegistry::new();
    registry.register(Arc::new(
        closeclaw_processor_chain::verbosity_filter::VerbosityFilter,
    ));
    registry.register(Arc::new(closeclaw_processor_chain::DslParser));
    registry
}

/// Setup gateway with a real processor chain and a session at the given
/// verbosity level. Returns (gateway, session_id).
async fn setup_verbosity_session(
    verbosity: closeclaw_common::VerbosityLevel,
    plugin: Arc<dyn IMPlugin>,
) -> (crate::Gateway, String) {
    let chain: Arc<dyn closeclaw_common::processor::ProcessorChain> =
        Arc::new(build_outbound_chain());
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::new(MockPersistService)),
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::with_processor_registry(config, Arc::clone(&sm), chain);
    gw.register_plugin(plugin).await;
    let msg = make_message("agent-1", "hello");
    let sid = sm.find_or_create("mock", &msg, None).await.unwrap();
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        sid.clone(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    cs_arc.write().await.set_verbosity_level(verbosity);
    sm.conversation_sessions
        .write()
        .await
        .insert(sid.clone(), cs_arc);
    (gw, sid)
}

/// Streaming events with a Thinking block followed by a Text block.
fn thinking_then_text_events() -> Vec<Result<StreamEvent, String>> {
    vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "internal reasoning".to_string(),
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
                text: "Visible answer.\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ]
}

/// At VerbosityLevel::Off, Thinking blocks are filtered in the incremental
/// phase (no plugin dispatch, no indicator). Only Text blocks are sent.
/// Post-stream VerbosityFilter is a no-op (no Thinking blocks remain).
#[tokio::test]
async fn test_streaming_verbosity_off_filters_thinking_sends_text() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) =
        setup_verbosity_session(closeclaw_common::VerbosityLevel::Off, plugin.clone()).await;

    let stream = stream::iter(thinking_then_text_events());
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // Step 1.2: In Off mode, Thinking blocks are filtered in the
    // incremental phase — no plugin dispatch, no thinking indicator.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        1,
        "only Text block should be sent during streaming in Off mode"
    );

    // Post-stream pipeline: no Thinking blocks remain to filter.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    let has_text = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Text(_)));
    assert!(
        !has_thinking,
        "Thinking block should be filtered from result at Off level"
    );
    assert!(has_text, "Text block should pass through at Off level");
}

/// At VerbosityLevel::Normal, Thinking blocks are filtered in the incremental
/// phase (no plugin dispatch, no indicator). Only Text blocks are sent.
/// Post-stream VerbosityFilter is a no-op (no Thinking blocks remain).
#[tokio::test]
async fn test_streaming_verbosity_normal_filters_thinking_post_stream() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) =
        setup_verbosity_session(closeclaw_common::VerbosityLevel::Normal, plugin.clone()).await;

    let stream = stream::iter(thinking_then_text_events());
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // Step 1.2: In Normal mode, Thinking blocks are filtered in the
    // incremental phase — no plugin dispatch, no thinking indicator.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        1,
        "only Text block should be sent during streaming in Normal mode"
    );

    // Post-stream pipeline: no Thinking blocks remain to filter.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    let has_text = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Text(_)));
    assert!(
        !has_thinking,
        "Thinking block should be filtered from result at Normal level"
    );
    assert!(has_text, "Text block should pass through at Normal level");
}

/// At VerbosityLevel::Full, Thinking blocks ARE sent via send_render_block
/// during streaming. Both Thinking and Text blocks appear in sent messages.
#[tokio::test]
async fn test_streaming_verbosity_full_sends_thinking_block() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) =
        setup_verbosity_session(closeclaw_common::VerbosityLevel::Full, plugin.clone()).await;

    let stream = stream::iter(thinking_then_text_events());
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // Step 1.1: In Full mode, Thinking blocks ARE sent via send_render_block
    // during streaming. Both Thinking and Text blocks should be sent.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        2,
        "both Thinking and Text should be sent during streaming in Full mode"
    );

    // Post-stream pipeline: VerbosityFilter keeps all blocks at Full level.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    let has_text = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Text(_)));
    assert!(
        has_thinking,
        "Thinking block should be in result at Full level"
    );
    assert!(has_text, "Text block should pass through at Full level");
}

// ═══════════════════════════════════════════════════════════════════════════
// Mixed Thinking + Text + DSL: streaming pipeline consistency tests
// (Step 1.3 — VerbosityFilter streaming consistency + DslParser passthrough)
// ═══════════════════════════════════════════════════════════════════════════

/// Streaming: Mixed Thinking + Text + DSL at Normal verbosity.
/// Thinking blocks are filtered in incremental phase (not sent).
/// Text + DSL lines are sent as-is (DslParser deferred to post-stream).
/// Post-stream: VerbosityFilter removes Thinking (none left), DslParser
/// strips DSL lines from content_blocks.
#[tokio::test]
async fn test_streaming_mixed_thinking_text_dsl_normal_verbosity() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) =
        setup_verbosity_session(closeclaw_common::VerbosityLevel::Normal, plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "internal reasoning".to_string(),
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
                text: "Please choose:\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: ContentDelta::Text {
                text: "::button[label:Yes;action:confirm;value:1]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // Normal: Thinking filtered in incremental phase — only Text lines sent.
    // LineBuffer produces 2 sends: one per newline-terminated line.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        2,
        "Text lines should be sent at Normal verbosity (1 per newline)"
    );
    assert_eq!(extract_text(&sent[0]), "Please choose:\n");
    assert_eq!(
        extract_text(&sent[1]),
        "::button[label:Yes;action:confirm;value:1]\n"
    );

    // Post-stream: no Thinking in result (filtered), DSL stripped from content.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(!has_thinking, "Thinking should be filtered at Normal level");

    // DslResult should contain the parsed instruction.
    let dsl = result
        .dsl_result
        .as_ref()
        .map(|s| serde_json::from_str::<closeclaw_common::processor::DslParseResult>(s).unwrap());
    assert!(dsl.is_some(), "dsl_result should be present");
    assert_eq!(dsl.unwrap().instructions.len(), 1);
}

/// Streaming: Mixed Thinking + Text + DSL at Full verbosity.
/// Thinking blocks are NOT filtered — sent via send_render_block.
/// Text + DSL lines sent as-is (DslParser deferred to post-stream).
/// Post-stream: VerbosityFilter keeps all, DslParser strips DSL.
#[tokio::test]
async fn test_streaming_mixed_thinking_text_dsl_full_verbosity() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) =
        setup_verbosity_session(closeclaw_common::VerbosityLevel::Full, plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "reasoning".to_string(),
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
                text: "Click below:\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: ContentDelta::Text {
                text: "::button[label:OK;action:submit;value:yes]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // Full: Thinking sent via send_render_block + Text sent via LineBuffer.
    // LineBuffer produces 2 sends (one per newline), Thinking produces 1 send.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        3,
        "Thinking (1) + Text (2 from LineBuffer) should be sent at Full verbosity"
    );

    // Post-stream: Thinking preserved at Full level.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(has_thinking, "Thinking should be preserved at Full level");

    // DslResult should contain the parsed instruction.
    let dsl = result
        .dsl_result
        .as_ref()
        .map(|s| serde_json::from_str::<closeclaw_common::processor::DslParseResult>(s).unwrap());
    assert!(dsl.is_some(), "dsl_result should be present");
    assert_eq!(dsl.unwrap().instructions.len(), 1);
}

/// Streaming: Mixed Thinking + Text + DSL at Off verbosity.
/// Thinking blocks filtered, only Text sent.
/// DSL lines sent as-is during streaming, parsed post-stream.
#[tokio::test]
async fn test_streaming_mixed_thinking_text_dsl_off_verbosity() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) =
        setup_verbosity_session(closeclaw_common::VerbosityLevel::Off, plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "hidden".to_string(),
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
                text: "::button[label:Go;action:run;value:1]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // Off: Thinking filtered, only DSL Text line sent.
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        1,
        "only Text line should be sent at Off verbosity"
    );
    assert_eq!(
        extract_text(&sent[0]),
        "::button[label:Go;action:run;value:1]\n"
    );

    // Post-stream: no Thinking, DSL stripped.
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(!has_thinking, "Thinking should be filtered at Off level");

    let dsl = result
        .dsl_result
        .as_ref()
        .map(|s| serde_json::from_str::<closeclaw_common::processor::DslParseResult>(s).unwrap());
    assert!(dsl.is_some(), "dsl_result should be present");
    assert_eq!(dsl.unwrap().instructions.len(), 1);
}

/// Streaming: DSL instruction line appears mid-stream mixed with regular text.
/// All lines sent as-is during streaming. Post-stream DslParser extracts
/// DSL instructions and strips them from content_blocks.
#[tokio::test]
async fn test_streaming_dsl_instruction_mixed_with_text_lines() {
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, sid) =
        setup_verbosity_session(closeclaw_common::VerbosityLevel::Normal, plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Line 1\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "::button[label:Click;action:go;value:ok]\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Line 3\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(default_usage()),
            finish_reason: Some("stop".to_string()),
        }),
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    // All 3 lines sent as-is during streaming.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 3, "all 3 lines should be sent");

    // Post-stream: DSL parsed, instruction extracted.
    let dsl = result
        .dsl_result
        .as_ref()
        .map(|s| serde_json::from_str::<closeclaw_common::processor::DslParseResult>(s).unwrap());
    assert!(dsl.is_some(), "dsl_result should be present");
    let instructions = dsl.unwrap().instructions;
    assert_eq!(instructions.len(), 1, "should extract 1 DSL instruction");
    assert_eq!(instructions[0].instruction_type, "button");
    assert_eq!(instructions[0].params["label"], "Click");
}

/// Streaming: VerbosityFilter consistency — batch filter and streaming
/// per-block filter produce the same result for mixed content.
/// This verifies the plan requirement: "流式路径和批量路径对相同输入产生相同过滤结果".
#[tokio::test]
async fn test_streaming_batch_consistency_verbos_filter() {
    use closeclaw_common::VerbosityLevel;

    // Same input blocks for both paths.
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "reasoning".to_string(),
            signature: None,
        },
        ContentBlock::Text("Hello".to_string()),
        ContentBlock::Thinking {
            thinking: "more reasoning".to_string(),
            signature: None,
        },
        ContentBlock::Text("World".to_string()),
        ContentBlock::ToolUse {
            id: "c1".into(),
            name: "tool".into(),
            input: "{}".into(),
        },
    ];

    let levels = [
        VerbosityLevel::Full,
        VerbosityLevel::Normal,
        VerbosityLevel::Off,
    ];

    for level in levels {
        // Batch path: full filter
        let batch_result = closeclaw_processor_chain::verbosity_filter::VerbosityFilter::filter(
            blocks.clone(),
            level,
        );

        // Streaming path: per-block should_keep_block filter
        let streaming_result: Vec<_> = blocks
            .iter()
            .filter(|b| {
                closeclaw_processor_chain::verbosity_filter::VerbosityFilter::should_keep_block(
                    b, level,
                )
            })
            .cloned()
            .collect();

        assert_eq!(
            batch_result.len(),
            streaming_result.len(),
            "block count mismatch at {:?}",
            level
        );

        // Verify the same blocks are kept (by discriminant)
        for (batch_block, stream_block) in batch_result.iter().zip(streaming_result.iter()) {
            assert_eq!(
                std::mem::discriminant(batch_block),
                std::mem::discriminant(stream_block),
                "block type mismatch at {:?}",
                level
            );
        }
    }
}
