//! Shutdown progress card rendering for the Gateway.
//!
//! Provides `send_shutdown_progress_card()` and `send_shutdown_final_card()`
//! for displaying real-time shutdown status to users via IM adapters.

use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::shutdown::ShutdownMode;
use serde_json::json;

use super::Gateway;

impl Gateway {
    /// Build and send a shutdown progress card to all active session chats.
    ///
    /// Displays per-session status (LLM streaming / tool executing / idle)
    /// and elapsed wait time. The card includes [Continue waiting] and
    /// [Force close] buttons. Sending failures are logged as warnings and
    /// do not block the shutdown flow.
    ///
    /// When `mode` is [`ShutdownMode::Forceful`], the header changes to
    /// indicate forced shutdown and the action buttons are omitted.
    pub async fn send_shutdown_progress_card(&self, mode: ShutdownMode) {
        use closeclaw_llm::session_state::LlmState;

        let sessions = self.session_manager.get_all_sessions().await;
        if sessions.is_empty() {
            return;
        }

        // First pass: group sessions by chat_id, drop read lock before second pass.
        let mut chats: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for session in &sessions {
            if let Some(chat_id) = self.session_manager.get_chat_id(&session.id).await {
                chats.entry(chat_id).or_default().push(session.id.clone());
            }
        }

        let mut session_elements: Vec<serde_json::Value> = Vec::new();
        for session in &sessions {
            // Re-acquire conv_sessions read lock per session to avoid
            // holding it across the entire loop (fixes E2 review item 1).
            let conv_sessions = self.session_manager.conversation_sessions.read().await;
            let (status_text, activity_at) = match conv_sessions.get(&session.id) {
                Some(cs) => {
                    let guard = cs.read().await;
                    let state = *guard.llm_state.read().expect("llm_state lock poisoned");
                    let activity = guard.last_activity_at();
                    let has_running_tool = {
                        let tool_states =
                            guard.tool_states.read().expect("tool_states lock poisoned");
                        tool_states.values().any(|s| {
                            matches!(
                                s,
                                closeclaw_llm::session_state::ToolExecState::RunningForeground
                                    | closeclaw_llm::session_state::ToolExecState::RunningBackground
                            )
                        })
                    };
                    drop(guard);
                    let label = if has_running_tool {
                        "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}"
                    } else if matches!(state, LlmState::Requesting | LlmState::Receiving) {
                        "LLM \u{6d41}\u{5f0f}\u{8f93}\u{51fa}\u{4e2d}"
                    } else {
                        "\u{5df2}\u{5c31}\u{7eea}"
                    };
                    (label, activity)
                }
                None => ("\u{5df2}\u{5c31}\u{7eea}", session.created_at),
            };
            drop(conv_sessions);

            let elapsed = {
                let now = chrono::Utc::now().timestamp();
                let secs = (now - activity_at).max(0) as u64;
                if secs < 60 {
                    format!("{}s", secs)
                } else {
                    format!("{}m{}s", secs / 60, secs % 60)
                }
            };

            session_elements.push(json!({
                "tag": "div",
                "text": json!({
                    "tag": "lark_md",
                    "content": format!(
                        "\u{2022} `{}` \u{2014} {} (\u{5df2}\u{7b49}\u{5f85} {})",
                        session.id, status_text, elapsed
                    )
                })
            }));
        }

        // Action buttons (only in graceful mode)
        let mut elements = session_elements;
        if mode == ShutdownMode::Graceful {
            elements.push(json!({
                "tag": "action",
                "actions": [
                    json!({
                        "tag": "button",
                        "text": json!({
                            "tag": "plain_text",
                            "content": "\u{7ee7}\u{7eed}\u{7b49}\u{5f85}"
                        }),
                        "type": "default",
                        "disabled": true
                    }),
                    json!({
                        "tag": "button",
                        "text": json!({
                            "tag": "plain_text",
                            "content": "\u{5f3a}\u{5236}\u{5173}\u{95ed}"
                        }),
                        "type": "danger",
                        "value": {"action": "forceful_shutdown"}
                    })
                ]
            }));
        }

        let header_title = if mode == ShutdownMode::Graceful {
            "\u{23f3} \u{6b63}\u{5728}\u{4f18}\u{96c5}\u{5173}\u{95ed}..."
        } else {
            "\u{26a0}\u{fe0f} \u{5f3a}\u{5236}\u{5173}\u{95ed}\u{4e2d}..."
        };

        let card = json!({
            "config": { "wide_screen_mode": true },
            "header": json!({
                "title": json!({
                    "tag": "plain_text",
                    "content": header_title
                }),
                "template": if mode == ShutdownMode::Graceful { "blue" } else { "red" }
            }),
            "elements": elements
        });

        // Send one card per chat (deduplicated by chat_id).
        let plugins = self.get_all_plugins().await;
        for chat_id in chats.keys() {
            for plugin in &plugins {
                let output = RenderedOutput {
                    msg_type: "interactive".into(),
                    payload: card.clone(),
                };
                if let Err(e) = plugin.send(&output, chat_id, None).await {
                    tracing::warn!(
                        chat_id = %chat_id,
                        plugin = plugin.platform(),
                        error = %e,
                        "failed to send shutdown progress card \u{2014} continuing"
                    );
                }
            }
        }
    }

    /// Send a final shutdown progress card indicating completion.
    pub async fn send_shutdown_final_card(
        &self,
        result: &crate::session_manager::stop::StopResult,
    ) {
        let sessions = self.session_manager.get_all_sessions().await;
        if sessions.is_empty() {
            return;
        }

        let mut chats: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for session in &sessions {
            if let Some(chat_id) = self.session_manager.get_chat_id(&session.id).await {
                chats.entry(chat_id).or_default().push(session.id.clone());
            }
        }
        if chats.is_empty() {
            return;
        }

        let summary = format!(
            "\u{2705} \u{5173}\u{95ed}\u{5b8c}\u{6210}\u{ff1a} {} \u{6210}\u{529f}, {} \u{5931}\u{8d25}, {} \u{8df3}\u{8fc7}",
            result.succeeded, result.failed, result.skipped
        );

        let card = json!({
            "config": { "wide_screen_mode": true },
            "header": json!({
                "title": json!({
                    "tag": "plain_text",
                    "content": "\u{2705} \u{5173}\u{95ed}\u{5b8c}\u{6210}"
                }),
                "template": "green"
            }),
            "elements": [
                json!({
                    "tag": "div",
                    "text": json!({
                        "tag": "lark_md",
                        "content": summary
                    })
                })
            ]
        });

        let plugins = self.get_all_plugins().await;
        for chat_id in chats.keys() {
            for plugin in &plugins {
                let output = RenderedOutput {
                    msg_type: "interactive".into(),
                    payload: card.clone(),
                };
                if let Err(e) = plugin.send(&output, chat_id, None).await {
                    tracing::warn!(
                        chat_id = %chat_id,
                        plugin = plugin.platform(),
                        error = %e,
                        "failed to send shutdown final card \u{2014} continuing"
                    );
                }
            }
        }
    }
}
