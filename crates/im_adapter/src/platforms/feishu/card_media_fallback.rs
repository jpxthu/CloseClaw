//! Card media fallback — send media elements when card send fails.
//!
//! When `send_card_json` fails (e.g. unsupported component), the fallback
//! path extracts text and media elements from the card payload and sends
//! them separately via text message API and `dispatch_send_media`.

use closeclaw_common::processor::ContentBlock;
use closeclaw_gateway::Message;
use std::collections::HashMap;
use tracing::warn;

use super::renderer::extract_card_plain_text;
use crate::IMAdapter;

/// Dispatch a standalone media message (image, file, or audio) directly
/// via lark-cli, bypassing card rendering.
///
/// Used by the text fallback path when card send fails — media elements
/// are sent directly via `lark-cli im +messages-send --image/--file`.
pub(super) async fn dispatch_send_media(
    adapter: &super::FeishuAdapter,
    peer_id: &str,
    block: &ContentBlock,
) -> Result<(), crate::error::AdapterError> {
    match block {
        ContentBlock::Image { name, url } => {
            if url.is_empty() {
                warn!(peer_id = %peer_id, name = %name,
                    "Image block has empty URL, skipping");
                return Ok(());
            }
            if let Err(e) = adapter.send_image(peer_id, url).await {
                warn!(peer_id = %peer_id, error = %e, name = %name,
                    "Failed to send image via lark-cli");
            }
            Ok(())
        }
        ContentBlock::Audio { name, url } | ContentBlock::File { name, url } => {
            if url.is_empty() {
                warn!(peer_id = %peer_id, name = %name,
                    "Audio/File block has empty URL, skipping");
                return Ok(());
            }
            if let Err(e) = adapter.send_file(peer_id, url).await {
                warn!(peer_id = %peer_id, error = %e, name = %name,
                    "Failed to send file via lark-cli");
            }
            Ok(())
        }
        _ => {
            warn!("dispatch_send_media called with non-media block");
            Ok(())
        }
    }
}

/// Extract and send media elements (img, audio, file) from a card payload.
///
/// Iterates over the card's `elements` array, identifies media tags,
/// and dispatches each via `dispatch_send_media`.
/// Logs warnings on individual element failures without aborting the loop.
pub(super) async fn send_media_from_card(
    adapter: &super::FeishuAdapter,
    peer_id: &str,
    output: &super::RenderedOutput,
) {
    let elements = match output
        .payload
        .get("card")
        .and_then(|c| c.get("elements"))
        .and_then(|e| e.as_array())
    {
        Some(e) => e,
        None => return,
    };

    for element in elements {
        let tag = element.get("tag").and_then(|t| t.as_str()).unwrap_or("");
        let block = match tag {
            "img" => element
                .get("img_key")
                .and_then(|k| k.as_str())
                .filter(|k| !k.is_empty())
                .map(|k| ContentBlock::Image {
                    name: String::new(),
                    url: k.to_string(),
                }),
            "audio" => element
                .get("file_token")
                .and_then(|k| k.as_str())
                .filter(|k| !k.is_empty())
                .map(|k| ContentBlock::Audio {
                    name: String::new(),
                    url: k.to_string(),
                }),
            _ => element
                .get("file_token")
                .and_then(|k| k.as_str())
                .filter(|k| !k.is_empty())
                .map(|k| ContentBlock::File {
                    name: String::new(),
                    url: k.to_string(),
                }),
        };
        if let Some(block) = block {
            if let Err(e) = dispatch_send_media(adapter, peer_id, &block).await {
                warn!(peer_id = %peer_id, error = %e,
                    "Failed to send media in fallback");
            }
        }
    }
}

/// Fallback: extract plain text and media from an interactive card
/// and send them via text message API and `dispatch_send_media`.
/// Logs warnings on failure and always returns so the Agent keeps running.
pub(super) async fn send_interactive_fallback(
    adapter: &super::FeishuAdapter,
    peer_id: &str,
    output: &super::RenderedOutput,
    thread_id: Option<&str>,
) {
    let plain_text = extract_card_plain_text(&output.payload);
    if !plain_text.is_empty() {
        let fallback = make_text_message(peer_id, &plain_text);
        if let Err(e2) = adapter.send_message(&fallback, thread_id).await {
            warn!(
                peer_id = %peer_id,
                error = %e2,
                "Feishu text fallback also failed — returning Ok(()) per design doc"
            );
        }
    }
    send_media_from_card(adapter, peer_id, output).await;
}

/// Build a text-mode [`Message`] targeting `peer_id`.
pub(super) fn make_text_message(peer_id: &str, text: &str) -> Message {
    Message {
        id: String::new(),
        from: String::new(),
        to: peer_id.to_string(),
        content: text.to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}
