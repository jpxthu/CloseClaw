//! Feishu (Lark) platform module: submodule declarations and re-exports.

mod adapter;
#[cfg(test)]
mod adapter_emoji_tests;
#[cfg(test)]
mod adapter_file_tests;
#[cfg(test)]
mod adapter_sticker_tests;
#[cfg(test)]
mod adapter_tests;
mod card_media;
pub(crate) mod card_media_fallback;
#[cfg(test)]
mod card_media_tests;
pub(crate) mod cardkit_streaming;
pub mod cleaner;
#[cfg(test)]
mod cleaner_tests;
pub(crate) mod config;
#[cfg(test)]
mod credential_isolation_tests;
mod debug_events;
#[cfg(test)]
mod debug_log_tests;
mod event_dedup;
mod events;
#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod feishu_adapter_tests;
#[cfg(test)]
mod feishu_tests;
mod identity;
#[cfg(test)]
mod identity_isolation_tests;
#[cfg(test)]
mod identity_resolver_stub;
mod inbound;
#[cfg(test)]
mod media_filter_tests;
#[cfg(test)]
mod normalize_cli_event_tests;
mod outbound_media;
#[cfg(test)]
mod outbound_media_tests;
mod plugin;
mod post_expand;
pub(crate) mod process_manager;
#[cfg(test)]
mod process_manager_tests;
#[cfg(test)]
mod register_injection_tests;
mod render_dispatch;
pub mod renderer;
#[cfg(test)]
mod renderer_decision_tests;
mod send_dispatch;
#[cfg(test)]
mod send_fallback_tests;
mod send_helpers;
#[cfg(test)]
mod send_warn_tests;
#[cfg(test)]
mod step1_8_gap_tests;
#[cfg(test)]
mod streaming_render_tests;
pub(crate) mod streaming_send;
#[cfg(test)]
mod style_tests;
mod text_style;
pub mod tools;
#[cfg(test)]
mod trace_id_tests;
#[cfg(test)]
mod try_resolve_media_path_tests;
pub(crate) mod xml_content;

pub use adapter::FeishuAdapter;
pub use plugin::{register, FeishuPlugin};
pub use renderer::build_text;
pub use renderer::should_use_card_for_blocks;

// Re-export adapter internals for test modules.
#[cfg(test)]
pub(crate) use adapter::{
    truncate_to_500, FeishuEvent, FeishuHeader, FeishuMessageEvent, FeishuSender, FeishuSenderId,
};
#[cfg(test)]
pub(crate) use post_expand::expand_post_content;
#[cfg(test)]
pub(crate) use renderer::extract_card_plain_text;
