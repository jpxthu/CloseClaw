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
mod lark_types;
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
#[cfg(test)]
mod renderer_tests;
mod send_dispatch;
#[cfg(test)]
mod send_fallback_tests;
mod send_helpers;
#[cfg(test)]
mod send_helpers_tests;
#[cfg(test)]
mod send_warn_tests;
#[cfg(test)]
mod step1_8_gap_tests;
#[cfg(test)]
mod streaming_render_tests;
pub(crate) mod streaming_send;
mod streaming_send_card;
#[cfg(test)]
mod streaming_send_tests;
#[cfg(test)]
mod style_tests;
#[cfg(test)]
mod test_helpers;
#[cfg(test)]
mod test_helpers_tests;
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

// Re-export adapter and lark_types internals for test modules.
#[cfg(test)]
pub(crate) use adapter::truncate_to_500;
#[cfg(test)]
pub(crate) use lark_types::{
    FeishuEvent, FeishuHeader, FeishuMessageEvent, FeishuSender, FeishuSenderId,
};
#[cfg(test)]
pub(crate) use post_expand::expand_post_content;
#[cfg(test)]
pub(crate) use renderer::extract_card_plain_text;
#[cfg(test)]
pub(crate) use test_helpers::{
    assert_button, assert_select_menu, assert_single_action, create_echo_cli,
    create_failing_mock_cli, create_mock_cli, create_subdir, make_adapter,
    make_adapter_with_profile, make_adapter_with_store, make_plugin, make_plugin_with_store,
    make_selector_inst, make_test_media_store, make_test_media_store_at,
    make_test_media_store_with_tmp, write_mock_script, CAPABILITY_ERROR_JSON,
};
