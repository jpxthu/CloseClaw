//! Additional streaming pipeline tests (part 3) for Gap 3 changes.
//!
//! Covers:
//! - Image/Audio/File blocks skip streaming renderer, collected by Gateway
//! - Image/Audio/File blocks still appear in final content_blocks
//! - Mixed streaming with Text + Image blocks

use super::*;

// ═══════════════════════════════════════════════════════════════════════════
// Gap 3: Image/Audio/File blocks skip streaming rendering
// ═══════════════════════════════════════════════════════════════════════════

/// Image blocks are NOT sent via send_render_block during streaming.
/// They are collected directly into content_blocks by Gateway.
#[tokio::test]
async fn test_streaming_image_block_not_sent_via_render() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Image,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ImageRef {
                name: "photo.jpg".to_string(),
                url: "https://cdn.example.com/photo.jpg".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Image,
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

    // Image block should NOT be sent via plugin.send (no render).
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        0,
        "Image block should not be sent via send_render_block"
    );

    // Image block should be in content_blocks (collected by Gateway).
    let has_image = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Image { .. }));
    assert!(has_image, "Image block should be in content_blocks");
}

/// Audio blocks are NOT sent via send_render_block during streaming.
#[tokio::test]
async fn test_streaming_audio_block_not_sent_via_render() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Audio,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::AudioRef {
                name: "recording.wav".to_string(),
                url: "https://cdn.example.com/recording.wav".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Audio,
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
    assert_eq!(
        sent.len(),
        0,
        "Audio block should not be sent via send_render_block"
    );

    let has_audio = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Audio { .. }));
    assert!(has_audio, "Audio block should be in content_blocks");
}

/// File blocks are NOT sent via send_render_block during streaming.
#[tokio::test]
async fn test_streaming_file_block_not_sent_via_render() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::File,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::FileRef {
                name: "report.pdf".to_string(),
                url: "https://cdn.example.com/report.pdf".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::File,
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
    assert_eq!(
        sent.len(),
        0,
        "File block should not be sent via send_render_block"
    );

    let has_file = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::File { .. }));
    assert!(has_file, "File block should be in content_blocks");
}

/// Mixed stream: Text + Image blocks. Text is sent, Image is not.
/// Both appear in content_blocks.
#[tokio::test]
async fn test_streaming_mixed_text_and_image() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        // Text block
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::Text {
                text: "Here is an image:\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Text,
        }),
        // Image block
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: ContentBlockType::Image,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: ContentDelta::ImageRef {
                name: "photo.jpg".to_string(),
                url: "https://cdn.example.com/photo.jpg".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::Image,
        }),
        // Another Text block
        Ok(StreamEvent::BlockStart {
            index: 2,
            block_type: ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 2,
            delta: ContentDelta::Text {
                text: "Done\n".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 2,
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

    // Only Text blocks are sent via plugin.send (2 sends).
    let sent = plugin.drain_sent();
    assert_eq!(
        sent.len(),
        2,
        "only Text blocks should be sent via send_render_block"
    );
    assert_eq!(extract_text(&sent[0]), "Here is an image:\n");
    assert_eq!(extract_text(&sent[1]), "Done\n");

    // All blocks (Text + Image) are in content_blocks.
    let has_text = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Text(_)));
    let has_image = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Image { .. }));
    assert!(has_text, "Text blocks should be in content_blocks");
    assert!(has_image, "Image block should be in content_blocks");
    assert_eq!(result.content_blocks.len(), 3, "2 Text + 1 Image");
}

/// All three media block types in one stream — none sent, all collected.
#[tokio::test]
async fn test_streaming_all_media_types_not_sent() {
    let chain = Arc::new(MockProcessorChain::new());
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    let (gw, _sm, sid) = setup_streaming(chain.clone(), plugin.clone()).await;

    let events = vec![
        Ok::<_, String>(StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Image,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: ContentDelta::ImageRef {
                name: "img.png".to_string(),
                url: "https://x.com/img.png".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: ContentBlockType::Image,
        }),
        Ok(StreamEvent::BlockStart {
            index: 1,
            block_type: ContentBlockType::Audio,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 1,
            delta: ContentDelta::AudioRef {
                name: "audio.mp3".to_string(),
                url: "https://x.com/audio.mp3".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 1,
            block_type: ContentBlockType::Audio,
        }),
        Ok(StreamEvent::BlockStart {
            index: 2,
            block_type: ContentBlockType::File,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 2,
            delta: ContentDelta::FileRef {
                name: "doc.pdf".to_string(),
                url: "https://x.com/doc.pdf".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 2,
            block_type: ContentBlockType::File,
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

    // No media blocks sent.
    let sent = plugin.drain_sent();
    assert_eq!(sent.len(), 0, "no media blocks should be sent");

    // All 3 media blocks collected in content_blocks.
    assert_eq!(result.content_blocks.len(), 3);
    assert!(result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Image { .. })));
    assert!(result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Audio { .. })));
    assert!(result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::File { .. })));
}
