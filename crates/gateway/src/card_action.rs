//! Card-action event handling for the Gateway.
//!
//! Card actions (button clicks, selector picks, etc.) bypass the
//! inbound Processor Chain and are injected directly as tool-result
//! payloads into the matching session's conversation context.

use super::Gateway;
use closeclaw_common::CardActionEvent;

impl Gateway {
    /// Handle a card-action event (button click, selector pick, etc.).
    ///
    /// Card actions bypass the inbound Processor Chain and are injected
    /// directly as tool-result payloads into the matching session's
    /// conversation context.
    ///
    /// Special case: `"forceful_shutdown"` escalates to forceful shutdown
    /// via the shutdown handle (global, no session injection).
    pub async fn handle_card_action(&self, action: CardActionEvent) {
        // ── Forceful shutdown: global action ───────────────────────
        if action.action_value == "forceful_shutdown" {
            if let Some(sh) = self.get_shutdown_handle() {
                if sh.escalate_to_forceful() {
                    tracing::info!(
                        sender_id = %action.sender_id,
                        "card action: escalating to forceful shutdown"
                    );
                }
            }
            return;
        }

        // ── Find the target session ────────────────────────────────
        let chat_id = action
            .metadata
            .get("chat_id")
            .map(|s| s.as_str())
            .unwrap_or("");

        let sessions = self.session_manager.get_all_sessions().await;
        let target = sessions
            .iter()
            .find(|s| s.channel == action.platform && (chat_id.is_empty() || s.agent_id == chat_id))
            .map(|s| s.id.clone());

        let Some(session_id) = target else {
            tracing::warn!(
                sender_id = %action.sender_id,
                action_value = %action.action_value,
                "card action: no matching session found — skipping"
            );
            return;
        };

        // ── Inject tool result into conversation ───────────────────
        let tool_call_id = format!("card_{}", action.action_value);
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(&session_id)
            .await
        {
            let mut cs = cs.write().await;
            cs.inject_tool_result(&tool_call_id, &action.action_value);
            tracing::info!(
                session_id = %session_id,
                action_value = %action.action_value,
                "card action: injected tool result"
            );
        } else {
            tracing::warn!(
                session_id = %session_id,
                "card action: conversation session not found — skipping"
            );
        }
    }
}
