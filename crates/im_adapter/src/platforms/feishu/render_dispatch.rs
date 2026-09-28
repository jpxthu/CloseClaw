//! Render dispatch — decide text vs card output and build the `RenderedOutput`.
//!
//! Extracted from `plugin.rs` to keep impl blocks within the 100-line limit.

use std::time::Instant;

use closeclaw_common::processor::ContentBlock;
use closeclaw_common::RenderedOutput;

use super::renderer::{self, build_card};
use super::{build_text, should_use_card_for_blocks, FeishuPlugin};

impl FeishuPlugin {
    /// Render content blocks into a Feishu output (plain text or card).
    ///
    /// Body of `IMPlugin::render`, extracted from `plugin.rs` so the trait
    /// impl block stays within the 100-line limit; the trait method delegates
    /// here with an unchanged signature.
    pub(super) fn render_blocks(&self, content_blocks: &[ContentBlock]) -> RenderedOutput {
        if content_blocks.is_empty() {
            return build_text("");
        }

        if content_blocks.len() == 1 {
            if let ContentBlock::Text(text) = &content_blocks[0] {
                if !renderer::should_use_card(text, false) {
                    return build_text(text.trim());
                }
            }
        }

        if !should_use_card_for_blocks(content_blocks, false) {
            return build_text("");
        }

        let start = Instant::now();
        let (title, elements) = renderer::dispatch_blocks(content_blocks, None, true);
        let output = build_card(title, elements);
        let render_duration_ms = start.elapsed().as_millis() as u64;

        // Emit structured debug_log event for outbound render.
        self.emit_debug_event(
            "outbound.render",
            serde_json::json!({
                "platform": "feishu",
                "msg_type": output.msg_type,
                "render_duration_ms": render_duration_ms,
            }),
        );

        output
    }
}
