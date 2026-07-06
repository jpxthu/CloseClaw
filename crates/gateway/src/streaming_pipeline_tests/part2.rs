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

/// DSL results from multiple streaming lines accumulate in StreamState.
/// After the stream ends, they are merged into the post-stream output.
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

    let parsed = chain.parsed_lines();
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0], "Please choose:\n");
    assert_eq!(parsed[1], "::button[label:Yes;action:confirm;value:1]\n");
    assert_eq!(parsed[2], "::button[label:No;action:cancel;value:0]\n");

    let sent = plugin.drain_sent();
    // Only plain text sent (DSL stripped to empty, skipped).
    assert_eq!(sent.len(), 1, "only plain text sent");
    assert_eq!(extract_text(&sent[0]), "Please choose:\n");
}

/// Multiple DSL instructions from different streaming lines are all
/// accumulated. When no plain text exists, sent is empty but DSL
/// is still tracked.
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

    let parsed = chain.parsed_lines();
    assert_eq!(parsed.len(), 2);

    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 0, "DSL-only lines should not be sent");

    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks.len(), 0, "no text blocks for DSL-only lines");
}

/// After streaming completes, the post-stream `process_or_bypass`
/// runs on already-clean content_blocks. Streaming DSL results are
/// merged back to avoid losing DSL instructions extracted during
/// streaming.
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
    assert_eq!(sent.len(), 1, "only plain text sent (DSL stripped)");
    assert_eq!(extract_text(&sent[0]), "Choose an option:\n");

    let text_blocks: Vec<String> = result
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks, vec!["Choose an option:\n"]);

    for block in &result.content_blocks {
        if let ContentBlock::Text(t) = block {
            assert!(
                !t.contains("::button"),
                "DSL marker should be stripped: {}",
                t
            );
        }
    }
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

/// Thinking blocks at VerbosityLevel::Off are filtered (not sent,
/// not accumulated) while Text blocks pass through the full pipeline.
#[tokio::test]
async fn test_streaming_verbosity_off_filters_thinking_sends_text() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let cs = closeclaw_session::llm_session::ConversationSession::new(
        sid.clone(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        cs_arc
            .write()
            .await
            .set_verbosity_level(closeclaw_common::VerbosityLevel::Off);
    }
    {
        let mut conv = _sm.conversation_sessions.write().await;
        conv.insert(sid.clone(), cs_arc);
    }

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Thinking {
                thinking: "hidden reasoning".to_string(),
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
    ];
    let stream = stream::iter(events);
    let plugin_arc: Arc<dyn IMPlugin> = plugin.clone();
    let result = gw
        .send_outbound_streaming(&sid, "mock", stream, &plugin_arc)
        .await
        .unwrap();

    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 1, "only text should be sent");
    assert_eq!(extract_text(&sent[0]), "Visible answer.");

    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(
        !has_thinking,
        "Thinking block should be filtered at Off level"
    );

    let has_text = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Text(_)));
    assert!(has_text, "Text block should pass through at Off level");
}
