//! Reaction and bot-added event types and parsers for Feishu webhook events.
//!
//! These events do not produce [`NormalizedMessage`]; they are only logged as
//! observability signals.

use crate::error::AdapterError;
use closeclaw_common::NormalizedMessage;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Reaction event types
// ---------------------------------------------------------------------------

/// Reaction event payload (`reaction.created`).
///
/// This event is not converted into a `NormalizedMessage`; it is only
/// logged as an observability signal.
#[derive(Debug, Deserialize)]
pub(crate) struct FeishuReactionEvent {
    pub(crate) message_id: String,
    pub(crate) operator: FeishuReactionOperator,
    pub(crate) reaction_type: FeishuReactionType,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuReactionOperator {
    pub(crate) operator_id: Option<FeishuReactionOperatorId>,
    pub(crate) open_id: Option<String>,
    #[allow(dead_code)]
    pub(crate) operator_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuReactionOperatorId {
    pub(crate) open_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuReactionType {
    pub(crate) emoji_type: String,
}

// ---------------------------------------------------------------------------
// Bot added event types
// ---------------------------------------------------------------------------

/// Bot added to chat event payload (`bot.added`).
///
/// This event is not converted into a `NormalizedMessage`; it is only
/// logged as an observability signal. The chat session will be created
/// when the first message arrives.
#[derive(Debug, Deserialize)]
pub(crate) struct FeishuBotAddedEvent {
    pub(crate) chat_id: String,
    pub(crate) bot: FeishuBotInfo,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuBotInfo {
    pub(crate) open_id: String,
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

/// Parse a `reaction.created` event, log it, and return `Ok(None)`.
///
/// This event does not produce a `NormalizedMessage`; it is recorded as
/// an observability signal for downstream agent observability.
pub(crate) fn parse_reaction_event(
    raw: &serde_json::Value,
) -> Result<Option<NormalizedMessage>, AdapterError> {
    let event: FeishuReactionEvent =
        serde_json::from_value(raw["event"].clone()).map_err(|e| {
            AdapterError::InvalidPayload(e.to_string())
        })?;

    let operator_id = event
        .operator
        .operator_id
        .as_ref()
        .map(|id| id.open_id.as_str())
        .or(event.operator.open_id.as_deref())
        .unwrap_or("unknown");

    tracing::info!(
        platform = "feishu",
        event_type = "reaction.created",
        message_id = %event.message_id,
        reaction_type = %event.reaction_type.emoji_type,
        operator = %operator_id,
        "Feishu reaction event received"
    );

    Ok(None)
}

/// Parse a `bot.added` event, log it, and return `Ok(None)`.
///
/// This event does not produce a `NormalizedMessage`; it is recorded as
/// an observability signal. The chat session will be created on the
/// first message arrival.
pub(crate) fn parse_bot_added_event(
    raw: &serde_json::Value,
) -> Result<Option<NormalizedMessage>, AdapterError> {
    let event: FeishuBotAddedEvent =
        serde_json::from_value(raw["event"].clone()).map_err(|e| {
            AdapterError::InvalidPayload(format!(
                "bot.added event parse failed: {}",
                e
            ))
        })?;

    tracing::info!(
        platform = "feishu",
        event_type = "bot.added",
        chat_id = %event.chat_id,
        bot_open_id = %event.bot.open_id,
        "Feishu bot added to chat"
    );

    Ok(None)
}
