//! MiniMax-specific request plugin.
//!
//! Injects `reasoning_split` into [`InternalRequest::extra_body`], allowing the
//! Anthropic protocol layer to forward it to the MiniMax API.

use crate::plugin::ModelPlugin;
use crate::types::InternalRequest;
use serde_json::Value;

/// Plugin that enriches MiniMax requests with provider-specific parameters.
///
/// Currently handles `reasoning_split` injection so that thinking content is
/// returned as a structured `reasoning_details` array during multi-turn tool
/// calls.
pub struct MiniMaxPlugin;

impl ModelPlugin for MiniMaxPlugin {
    fn name(&self) -> &str {
        "minimax"
    }

    fn before_request(&self, request: &mut InternalRequest) {
        request
            .extra_body
            .insert("reasoning_split".to_string(), Value::Bool(true));
    }
}

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod plugin_tests;
