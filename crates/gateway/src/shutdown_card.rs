//! Shutdown progress card rendering for the Gateway.
//!
//! Provides `send_shutdown_progress_card()` and `send_shutdown_final_card()`
//! for displaying real-time shutdown status to users via IM adapters.

use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_llm::session_state::LlmState;
use serde_json::json;

use super::Gateway;

/// Build the session status label for display in the shutdown progress card.
///
/// When tools are running, displays the tool name and a truncated input
/// summary (max 30 chars). Falls back to generic labels for LLM streaming
/// or idle states.
fn build_session_status_label(
    has_running_tool: bool,
    tool_info: &[(String, String)],
    llm_state: LlmState,
) -> String {
    if has_running_tool {
        if let Some((tool_name, input)) = tool_info.first() {
            let input_brief: String = input.chars().take(30).collect();
            let display = if input_brief.len() < input.len() {
                format!("{}...", input_brief)
            } else {
                input_brief
            };
            if display.is_empty() {
                format!(
                    "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}{}",
                    tool_name
                )
            } else {
                format!(
                    "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}{} {}",
                    tool_name, display
                )
            }
        } else {
            "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}".to_string()
        }
    } else if matches!(llm_state, LlmState::Requesting | LlmState::Receiving) {
        "LLM \u{6d41}\u{5f0f}\u{8f93}\u{51fa}\u{4e2d}".to_string()
    } else {
        "\u{5df2}\u{5c31}\u{7eea}".to_string()
    }
}

impl Gateway {
    /// Send a brief initial shutdown notification (Phase 0).
    ///
    /// Displays a simple message like "⏳ 正在优雅关闭..." or
    /// "⚠️ 强制关闭中..." without per-session details. Session progress
    /// will be shown later in Phase 2 via [`send_shutdown_progress_card`].
    pub async fn send_shutdown_start_notification(&self, mode: ShutdownMode) {
        let header_title = if mode == ShutdownMode::Graceful {
            "⏳ 正在优雅关闭..."
        } else {
            "⚠️ 强制关闭中..."
        };

        let body = if mode == ShutdownMode::Graceful {
            "系统正在优雅关闭，drain 结束后将展示 session 进度详情。"
        } else {
            "系统正在强制关闭，未完成的操作可能需要手动恢复。"
        };

        let mut elements: Vec<serde_json::Value> = vec![json!({
            "tag": "div",
            "text": json!({
                "tag": "lark_md",
                "content": body
            })
        })];

        if mode == ShutdownMode::Graceful {
            elements.push(json!({
                "tag": "action",
                "actions": [
                    json!({
                        "tag": "button",
                        "text": json!({
                            "tag": "plain_text",
                            "content": "继续等待"
                        }),
                        "type": "default",
                        "disabled": true
                    }),
                    json!({
                        "tag": "button",
                        "text": json!({
                            "tag": "plain_text",
                            "content": "强制关闭"
                        }),
                        "type": "danger",
                        "value": {"action": "forceful_shutdown"}
                    })
                ]
            }));
        }

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
                        "failed to send shutdown start notification — continuing"
                    );
                }
            }
        }
    }

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
                    let (has_running_tool, running_ids) = {
                        let tool_states =
                            guard.tool_states.read().expect("tool_states lock poisoned");
                        let ids: Vec<String> = tool_states
                            .iter()
                            .filter(|(_, s)| {
                                matches!(
                                    *s,
                                    closeclaw_llm::session_state::ToolExecState::RunningForeground
                                        | closeclaw_llm::session_state::ToolExecState::RunningBackground
                                )
                            })
                            .map(|(id, _)| id.clone())
                            .collect();
                        (!ids.is_empty(), ids)
                    };

                    // Extract tool name and input from conversation messages
                    // for sessions with running tools.
                    let tool_info: Vec<(String, String)> = if has_running_tool {
                        let pending = guard.extract_pending_tool_calls();
                        pending
                            .into_iter()
                            .filter(|op| running_ids.contains(&op.op_id))
                            .map(|op| (op.name, op.args))
                            .collect()
                    } else {
                        Vec::new()
                    };

                    drop(guard);
                    let label = build_session_status_label(has_running_tool, &tool_info, state);
                    (label, activity)
                }
                None => ("\u{5df2}\u{5c31}\u{7eea}".to_string(), session.created_at),
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

    /// Send a heartbeat card during Phase 2 when no state changes for a while.
    ///
    /// Displays a simplified format: "⏳ 仍在关闭中，N 个 session 活跃（最长等待 Ns）"
    /// with [Continue waiting] and [Force close] buttons. Only sent in graceful
    /// mode. Sending failures are logged as warnings and do not block shutdown.
    pub async fn send_shutdown_heartbeat_card(
        &self,
        active_count: usize,
        longest_wait_secs: u64,
        mode: closeclaw_common::shutdown::ShutdownMode,
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

        let content = format!(
            "⏳ 仍在关闭中，{} 个 session 活跃（最长等待 {}s）",
            active_count, longest_wait_secs
        );

        let mut elements: Vec<serde_json::Value> = vec![json!({
            "tag": "div",
            "text": json!({
                "tag": "lark_md",
                "content": content
            })
        })];

        if mode == closeclaw_common::shutdown::ShutdownMode::Graceful {
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

        let card = json!({
            "config": { "wide_screen_mode": true },
            "header": json!({
                "title": json!({
                    "tag": "plain_text",
                    "content": "⏳ 心跳 — 关闭仍在进行中"
                }),
                "template": "blue"
            }),
            "elements": elements
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
                        "failed to send shutdown heartbeat card — continuing"
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

// ═════════════════════════════════════════════════════════════════════════════
// Unit tests
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_llm::session_state::LlmState;

    /// Build the start notification card JSON for a given mode.
    /// Extracted from `send_shutdown_start_notification` for testability.
    fn build_start_notification_card(mode: ShutdownMode) -> serde_json::Value {
        let header_title = if mode == ShutdownMode::Graceful {
            "\u{23f3} \u{6b63}\u{5728}\u{4f18}\u{96c5}\u{5173}\u{95ed}..."
        } else {
            "\u{26a0}\u{fe0f} \u{5f3a}\u{5236}\u{5173}\u{95ed}\u{4e2d}..."
        };
        let body = if mode == ShutdownMode::Graceful {
            "\u{7cfb}\u{7edf}\u{6b63}\u{5728}\u{4f18}\u{96c5}\u{5173}\u{95ed}\u{ff0c}drain \u{7ed3}\u{675f}\u{540e}\u{5c06}\u{5c55}\u{793a} session \u{8fdb}\u{5ea6}\u{8be6}\u{60c5}\u{3002}"
        } else {
            "\u{7cfb}\u{7edf}\u{6b63}\u{5728}\u{5f3a}\u{5236}\u{5173}\u{95ed}\u{ff0c}\u{672a}\u{5b8c}\u{6210}\u{7684}\u{64cd}\u{4f5c}\u{53ef}\u{80fd}\u{9700}\u{8981}\u{624b}\u{52a8}\u{6062}\u{590d}\u{3002}"
        };

        let mut elements: Vec<serde_json::Value> = vec![json!({
            "tag": "div",
            "text": json!({
                "tag": "lark_md",
                "content": body
            })
        })];

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

        json!({
            "config": { "wide_screen_mode": true },
            "header": json!({
                "title": json!({
                    "tag": "plain_text",
                    "content": header_title
                }),
                "template": if mode == ShutdownMode::Graceful { "blue" } else { "red" }
            }),
            "elements": elements
        })
    }

    // ── build_session_status_label tests ──────────────────────────────────────

    /// Running tool with name and input shows tool details.
    #[test]
    fn test_label_running_tool_with_name_and_input() {
        let tool_info = vec![("make".to_string(), "build --release".to_string())];
        let label = build_session_status_label(true, &tool_info, LlmState::Idle);
        assert_eq!(
            label,
            "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}make build --release"
        );
    }

    /// Running tool with long input is truncated to 30 chars.
    #[test]
    fn test_label_running_tool_long_input_truncated() {
        let long_input = "a".repeat(50);
        let tool_info = vec![("compile".to_string(), long_input)];
        let label = build_session_status_label(true, &tool_info, LlmState::Idle);
        assert!(label.starts_with("\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}compile "));
        // Should end with "..."
        assert!(label.ends_with("..."));
        // The input_brief part should be <= 33 chars (30 + "...")
        let prefix = "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}compile ";
        let brief_part = &label[prefix.len()..];
        assert!(brief_part.len() <= 33);
    }

    /// Running tool with empty input shows tool name only.
    #[test]
    fn test_label_running_tool_empty_input() {
        let tool_info = vec![("list_files".to_string(), "".to_string())];
        let label = build_session_status_label(true, &tool_info, LlmState::Idle);
        assert_eq!(
            label,
            "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}list_files"
        );
    }

    /// Running tool with no matching tool_info falls back to generic label.
    #[test]
    fn test_label_running_tool_no_info_fallback() {
        let tool_info: Vec<(String, String)> = Vec::new();
        let label = build_session_status_label(true, &tool_info, LlmState::Idle);
        assert_eq!(label, "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}");
    }

    /// No running tool, LLM requesting state.
    #[test]
    fn test_label_llm_requesting() {
        let tool_info: Vec<(String, String)> = Vec::new();
        let label = build_session_status_label(false, &tool_info, LlmState::Requesting);
        assert_eq!(label, "LLM \u{6d41}\u{5f0f}\u{8f93}\u{51fa}\u{4e2d}");
    }

    /// No running tool, LLM receiving state.
    #[test]
    fn test_label_llm_receiving() {
        let tool_info: Vec<(String, String)> = Vec::new();
        let label = build_session_status_label(false, &tool_info, LlmState::Receiving);
        assert_eq!(label, "LLM \u{6d41}\u{5f0f}\u{8f93}\u{51fa}\u{4e2d}");
    }

    /// No running tool, idle state.
    #[test]
    fn test_label_idle() {
        let tool_info: Vec<(String, String)> = Vec::new();
        let label = build_session_status_label(false, &tool_info, LlmState::Idle);
        assert_eq!(label, "\u{5df2}\u{5c31}\u{7eea}");
    }

    /// Running tool takes precedence over LLM streaming state.
    #[test]
    fn test_label_running_tool_over_llm_streaming() {
        let tool_info = vec![("exec".to_string(), "cargo test".to_string())];
        let label = build_session_status_label(true, &tool_info, LlmState::Receiving);
        assert!(label.starts_with("\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}"));
        assert!(label.contains("exec"));
    }

    /// Multi-byte UTF-8 input is truncated safely.
    #[test]
    fn test_label_running_tool_multibyte_input() {
        let input = "\u{4e2d}\u{6587}\u{6d4b}\u{8bd5}\u{5de5}\u{5177}\u{540d}\u{79f0}\u{548c}\u{53c2}\u{6570}".repeat(5);
        let tool_info = vec![("tool".to_string(), input)];
        let label = build_session_status_label(true, &tool_info, LlmState::Idle);
        assert!(label.starts_with("\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}tool "));
        // Should not panic on multi-byte truncation
        assert!(label.len() > 0);
    }

    // ── send_shutdown_start_notification card content tests ─────────────────

    /// Graceful mode start notification has blue header and body mentioning
    /// "drain" and "session 进度详情".
    #[test]
    fn test_start_notification_graceful_card_content() {
        let card = build_start_notification_card(ShutdownMode::Graceful);

        // Header should be blue
        assert_eq!(card["header"]["template"], "blue");
        // Title should mention 优雅关闭
        let title = card["header"]["title"]["content"].as_str().unwrap();
        assert!(title.contains("\u{4f18}\u{96c5}\u{5173}\u{95ed}"));

        // Body should mention drain and session 进度详情
        let body = card["elements"][0]["text"]["content"].as_str().unwrap();
        assert!(body.contains("drain"));
        assert!(body.contains("session"));
        assert!(body.contains("\u{8fdb}\u{5ea6}\u{8be6}\u{60c5}"));

        // Should have action buttons (Continue waiting + Force close)
        let actions = card["elements"][1]["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0]["type"], "default");
        assert_eq!(actions[1]["type"], "danger");
    }

    /// Forceful mode start notification has red header, no action buttons,
    /// and body mentions 强制关闭.
    #[test]
    fn test_start_notification_forceful_card_content() {
        let card = build_start_notification_card(ShutdownMode::Forceful);

        // Header should be red
        assert_eq!(card["header"]["template"], "red");
        // Title should mention 强制关闭
        let title = card["header"]["title"]["content"].as_str().unwrap();
        assert!(title.contains("\u{5f3a}\u{5236}\u{5173}\u{95ed}"));

        // Body should mention 强制关闭
        let body = card["elements"][0]["text"]["content"].as_str().unwrap();
        assert!(body.contains("\u{5f3a}\u{5236}\u{5173}\u{95ed}"));

        // Should NOT have action buttons (only 1 element: the body div)
        assert_eq!(card["elements"].as_array().unwrap().len(), 1);
    }

    /// Start notification card is brief — no per-session status details.
    #[test]
    fn test_start_notification_no_session_details() {
        let card_graceful = build_start_notification_card(ShutdownMode::Graceful);
        let card_forceful = build_start_notification_card(ShutdownMode::Forceful);

        // Neither card should contain session IDs or per-session status
        let body_g = card_graceful["elements"][0]["text"]["content"]
            .as_str()
            .unwrap();
        let body_f = card_forceful["elements"][0]["text"]["content"]
            .as_str()
            .unwrap();

        // No session- prefix (session IDs look like "session-1" etc.)
        assert!(!body_g.contains("session-"));
        assert!(!body_f.contains("session-"));
        // No tool execution details
        assert!(!body_g.contains("\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}"));
        assert!(!body_f.contains("\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}"));
        // No LLM streaming details
        assert!(!body_g.contains("LLM"));
        assert!(!body_f.contains("LLM"));
    }

    // ── Progress card tool display behavior tests ───────────────────────────

    /// When a tool is running with name "make" and input "build --release",
    /// the session status label shows tool name and truncated input.
    #[test]
    fn test_progress_card_tool_display_format() {
        let tool_info = vec![("make".to_string(), "build --release".to_string())];
        let label = build_session_status_label(true, &tool_info, LlmState::Idle);
        // Must match design doc format: "工具执行中：make build --release"
        assert_eq!(
            label,
            "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}make build --release"
        );
    }

    /// When no tools are running and LLM is idle, label is "已就绪"
    /// (not "工具执行中").
    #[test]
    fn test_progress_card_no_tool_shows_idle_not_tool_executing() {
        let tool_info: Vec<(String, String)> = Vec::new();
        let label = build_session_status_label(false, &tool_info, LlmState::Idle);
        assert_eq!(label, "\u{5df2}\u{5c31}\u{7eea}");
        assert!(!label.contains("\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}"));
    }

    /// Tool name with empty input: shows tool name only, no trailing space.
    #[test]
    fn test_progress_card_tool_empty_input_no_trailing_space() {
        let tool_info = vec![("web_search".to_string(), "".to_string())];
        let label = build_session_status_label(true, &tool_info, LlmState::Idle);
        assert_eq!(
            label,
            "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}web_search"
        );
        // No trailing space after tool name
        assert!(!label.ends_with(' '));
    }

    /// Tool with exactly 30-char input: no truncation, no "..." suffix.
    #[test]
    fn test_progress_card_tool_input_exactly_30_chars_no_truncation() {
        let input = "a".repeat(30);
        let tool_info = vec![("exec".to_string(), input.clone())];
        let label = build_session_status_label(true, &tool_info, LlmState::Idle);
        assert!(label.contains(&input));
        assert!(!label.ends_with("..."));
    }

    /// Tool with 31-char input: truncated to 30 chars + "...".
    #[test]
    fn test_progress_card_tool_input_31_chars_truncated() {
        let input = "b".repeat(31);
        let tool_info = vec![("exec".to_string(), input)];
        let label = build_session_status_label(true, &tool_info, LlmState::Idle);
        assert!(label.ends_with("..."));
        // The brief part before "..." should be 30 chars
        let suffix = "\u{5de5}\u{5177}\u{6267}\u{884c}\u{4e2d}\u{ff1a}exec ";
        let brief = &label[suffix.len()..label.len() - 3];
        assert_eq!(brief.len(), 30);
    }
}
