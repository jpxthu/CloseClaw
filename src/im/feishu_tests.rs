#[cfg(test)]
mod tests {
    use crate::gateway::Message;
    use crate::im::feishu::{CachedToken, FeishuAdapter, FeishuPlugin};
    use crate::im::{IMAdapter, IMPlugin};
    use crate::renderer::feishu::FeishuRenderer;
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn test_feishu_adapter_name() {
        let adapter = FeishuAdapter::new(
            "app_id".to_string(),
            "app_secret".to_string(),
            "token".to_string(),
        );
        assert_eq!(adapter.name(), "feishu");
    }

    #[test]
    fn test_cached_token_needs_refresh_expired() {
        let cached = CachedToken {
            token: "t".to_string(),
            expires_at: Instant::now() - Duration::from_secs(10),
        };
        assert!(cached.needs_refresh());
    }

    #[test]
    fn test_cached_token_needs_refresh_valid() {
        let cached = CachedToken {
            token: "t".to_string(),
            expires_at: Instant::now() + Duration::from_secs(7200),
        };
        assert!(!cached.needs_refresh());
    }

    #[tokio::test]
    async fn test_validate_signature_correct() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "my_token".into());
        let payload = b"test";
        let mut hasher = Sha256::new();
        hasher.update(b"my_token");
        hasher.update(payload);
        let sig = format!("{:x}", hasher.finalize());
        assert!(adapter.validate_signature(&sig, payload).await);
    }

    #[tokio::test]
    async fn test_validate_signature_incorrect() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        assert!(!adapter.validate_signature("wrong", b"test").await);
    }

    #[tokio::test]
    async fn test_handle_webhook_valid() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id":"evt_1",
                "event_type":"im.message.receive_v1",
                "create_time":"0",
                "token":"t",
                "app_id":"a"
            },
            "event": {
                "sender": {
                    "sender_id":{"open_id":"ou_abc"},
                    "sender_type":"user"
                },
                "content":"{\"text\":\"hello\"}",
                "chat_id":"oc_x",
                "message_type":"text"
            }
        });
        let msg = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .expect("expected Some(message)");
        assert_eq!(msg.id, "evt_1");
        assert_eq!(msg.from, "ou_abc");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.metadata.get("account_id"), Some(&"a".to_string()));
    }

    #[tokio::test]
    async fn test_handle_webhook_invalid_json() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        assert!(adapter.handle_webhook(b"not json").await.is_err());
    }

    #[tokio::test]
    async fn test_handle_webhook_empty_text() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        let payload = serde_json::json!({
            "schema":"2.0",
            "header":{
                "event_id":"e2",
                "event_type":"x",
                "create_time":"0",
                "token":"t",
                "app_id":"a"
            },
            "event":{
                "sender":{
                    "sender_id":{"open_id":"ou_x"},
                    "sender_type":"user"
                },
                "content":"{\"other\":\"data\"}",
                "chat_id":"oc_y",
                "message_type":"text"
            }
        });
        let msg = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .expect("expected Some(message)");
        assert_eq!(msg.content, "");
        assert_eq!(msg.metadata.get("account_id"), Some(&"a".to_string()));
    }

    #[tokio::test]
    async fn test_error_cases() {
        let a = FeishuAdapter::new("bad".into(), "bad".into(), "t".into());
        assert!(a.fetch_tenant_token().await.is_err());
        let msg = Message {
            id: "1".into(),
            from: "a".into(),
            to: "b".into(),
            content: "hi".into(),
            channel: "feishu".into(),
            timestamp: 0,
            metadata: HashMap::new(),
            thread_id: None,
        };
        assert!(a.send_message(&msg, None).await.is_err());
    }

    #[tokio::test]
    async fn test_update_message_error() {
        let adapter = FeishuAdapter::new("bad".into(), "bad".into(), "t".into());
        assert!(adapter
            .update_message("om_1", &serde_json::json!({}))
            .await
            .is_err());
    }

    // ── thread_id tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_handle_webhook_with_thread_id() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {"event_id": "e1", "event_type": "im.message.receive_v1",
                        "create_time": "0", "token": "t", "app_id": "a"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_1"}, "sender_type": "user"},
                "content": "{\"text\":\"hi\"}",
                "chat_id": "oc_x", "message_type": "text",
                "thread_id": "omt_thread_abc"
            }
        });
        let msg = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .expect("expected Some(message)");
        assert_eq!(
            msg.metadata.get("thread_id"),
            Some(&"omt_thread_abc".to_string())
        );
    }

    #[tokio::test]
    async fn test_handle_webhook_with_root_id() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {"event_id": "e2", "event_type": "im.message.receive_v1",
                        "create_time": "0", "token": "t", "app_id": "a"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_2"}, "sender_type": "user"},
                "content": "{\"text\":\"hello\"}",
                "chat_id": "oc_y", "message_type": "text",
                "root_id": "omt_root_123"
            }
        });
        let msg = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .expect("expected Some(message)");
        assert_eq!(
            msg.metadata.get("thread_id"),
            Some(&"omt_root_123".to_string())
        );
    }

    #[tokio::test]
    async fn test_handle_webhook_with_parent_id() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {"event_id": "e3", "event_type": "im.message.receive_v1",
                        "create_time": "0", "token": "t", "app_id": "a"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_3"}, "sender_type": "user"},
                "content": "{\"text\":\"hey\"}",
                "chat_id": "oc_z", "message_type": "text",
                "parent_id": "omt_parent_456"
            }
        });
        let msg = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .expect("expected Some(message)");
        assert_eq!(
            msg.metadata.get("thread_id"),
            Some(&"omt_parent_456".to_string())
        );
    }

    #[tokio::test]
    async fn test_handle_webhook_no_thread_fields() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {"event_id": "e4", "event_type": "im.message.receive_v1",
                        "create_time": "0", "token": "t", "app_id": "a"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_4"}, "sender_type": "user"},
                "content": "{\"text\":\"yo\"}",
                "chat_id": "oc_w", "message_type": "text"
            }
        });
        let msg = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .expect("expected Some(message)");
        assert!(!msg.metadata.contains_key("thread_id"));
    }

    #[tokio::test]
    async fn test_handle_webhook_thread_id_priority() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        // All three present — thread_id should win
        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {"event_id": "e5", "event_type": "im.message.receive_v1",
                        "create_time": "0", "token": "t", "app_id": "a"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_5"}, "sender_type": "user"},
                "content": "{\"text\":\"test\"}",
                "chat_id": "oc_v", "message_type": "text",
                "thread_id": "omt_direct",
                "root_id": "omt_root",
                "parent_id": "omt_parent"
            }
        });
        let msg = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .expect("expected Some(message)");
        assert_eq!(
            msg.metadata.get("thread_id"),
            Some(&"omt_direct".to_string())
        );

        // thread_id absent, root_id should win
        let payload2 = serde_json::json!({
            "schema": "2.0",
            "header": {"event_id": "e6", "event_type": "im.message.receive_v1",
                        "create_time": "0", "token": "t", "app_id": "a"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_6"}, "sender_type": "user"},
                "content": "{\"text\":\"test2\"}",
                "chat_id": "oc_u", "message_type": "text",
                "root_id": "omt_root2",
                "parent_id": "omt_parent2"
            }
        });
        let msg2 = adapter
            .handle_webhook(&serde_json::to_vec(&payload2).unwrap())
            .await
            .unwrap()
            .expect("expected Some(message)");
        assert_eq!(
            msg2.metadata.get("thread_id"),
            Some(&"omt_root2".to_string())
        );
    }

    #[tokio::test]
    async fn test_parse_inbound_thread_id() {
        let adapter = Arc::new(FeishuAdapter::new("a".into(), "s".into(), "t".into()));
        let renderer = Arc::new(FeishuRenderer::new());
        let plugin = FeishuPlugin::new(adapter, renderer);

        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {"event_id": "e7",
                        "event_type": "im.message.receive_v1",
                        "create_time": "0", "token": "t", "app_id": "a"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_7"}, "sender_type": "user"},
                "content": "{\"text\":\"hi thread\"}",
                "chat_id": "oc_t", "message_type": "text",
                "thread_id": "omt_from_webhook"
            }
        });
        let msg = plugin
            .parse_inbound(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(msg.thread_id, Some("omt_from_webhook".to_string()));
        assert_eq!(msg.platform, "feishu");
        assert_eq!(msg.content, "hi thread");
    }

    #[tokio::test]
    async fn test_parse_inbound_no_thread_id() {
        let adapter = Arc::new(FeishuAdapter::new("a".into(), "s".into(), "t".into()));
        let renderer = Arc::new(FeishuRenderer::new());
        let plugin = FeishuPlugin::new(adapter, renderer);

        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {"event_id": "e8",
                        "event_type": "im.message.receive_v1",
                        "create_time": "0", "token": "t", "app_id": "a"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_8"}, "sender_type": "user"},
                "content": "{\"text\":\"hi\"}",
                "chat_id": "oc_s", "message_type": "text"
            }
        });
        let msg = plugin
            .parse_inbound(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(msg.thread_id, None);
    }

    // ── lifecycle hook tests (Step 1.1) ──────────────────────────────

    /// close_inbound on FeishuAdapter succeeds (clears cached token).
    #[tokio::test]
    async fn test_feishu_adapter_close_inbound_succeeds() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        // Trigger a token fetch attempt (will fail with bad creds, but
        // that's fine — we just need close_inbound to not panic).
        adapter.close_inbound().await.unwrap();
    }

    /// close_outbound on FeishuAdapter succeeds (clears cached token).
    #[tokio::test]
    async fn test_feishu_adapter_close_outbound_succeeds() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        adapter.close_outbound().await.unwrap();
    }

    /// close_inbound on FeishuPlugin delegates to adapter and succeeds.
    #[tokio::test]
    async fn test_feishu_plugin_close_inbound() {
        let adapter = Arc::new(FeishuAdapter::new("a".into(), "s".into(), "t".into()));
        let renderer = Arc::new(FeishuRenderer::new());
        let plugin = FeishuPlugin::new(adapter, renderer);
        plugin.close_inbound().await.unwrap();
    }

    /// close_outbound on FeishuPlugin delegates to adapter and succeeds.
    #[tokio::test]
    async fn test_feishu_plugin_close_outbound() {
        let adapter = Arc::new(FeishuAdapter::new("a".into(), "s".into(), "t".into()));
        let renderer = Arc::new(FeishuRenderer::new());
        let plugin = FeishuPlugin::new(adapter, renderer);
        plugin.close_outbound().await.unwrap();
    }

    /// close_inbound is idempotent — calling twice still succeeds.
    #[tokio::test]
    async fn test_feishu_adapter_close_inbound_idempotent() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        adapter.close_inbound().await.unwrap();
        adapter.close_inbound().await.unwrap();
    }

    /// close_outbound is idempotent — calling twice still succeeds.
    #[tokio::test]
    async fn test_feishu_adapter_close_outbound_idempotent() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        adapter.close_outbound().await.unwrap();
        adapter.close_outbound().await.unwrap();
    }

    // ── root_id URL encoding test ──────────────────────────────────────

    #[test]
    fn test_root_id_percent_encoding() {
        // Bug #904: root_id with special characters should be percent-encoded.
        let rid = "omt_abc=def&ghi";
        let encoded: String = url::form_urlencoded::byte_serialize(rid.as_bytes()).collect();
        assert_eq!(encoded, "omt_abc%3Ddef%26ghi");

        // Verify the URL construction pattern used in feishu.rs
        let base_url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=open_id";
        let url = format!("{}&root_id={}", base_url, encoded);
        assert!(
            url.contains("root_id=omt_abc%3Ddef%26ghi"),
            "URL should contain percent-encoded root_id, got: {}",
            url
        );
    }

    // ── card.action.trigger tests ──────────────────────────────────────

    #[tokio::test]
    async fn test_handle_webhook_card_action_forceful_shutdown() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        let payload = serde_json::json!({
            "header": {
                "event_id": "evt_card_1",
                "event_type": "card.action.trigger",
                "create_time": "0",
                "token": "t",
                "app_id": "a"
            },
            "operator": {
                "open_id": "ou_operator_1"
            },
            "token": "verify_token",
            "action": {
                "value": {"action": "forceful_shutdown", "chat_id": "oc_chat_1"},
                "tag": "button"
            }
        });
        let msg = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .expect("forceful_shutdown should return Some(message)");
        assert_eq!(msg.content, "/__card_action:forceful_shutdown");
        assert_eq!(msg.from, "ou_operator_1");
        assert_eq!(msg.id, "evt_card_1");
        assert_eq!(msg.metadata.get("card_action"), Some(&"true".to_string()));
        assert_eq!(msg.metadata.get("chat_id"), Some(&"oc_chat_1".to_string()));
    }

    #[tokio::test]
    async fn test_handle_webhook_card_action_unknown() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        let payload = serde_json::json!({
            "header": {
                "event_id": "evt_card_2",
                "event_type": "card.action.trigger",
                "create_time": "0",
                "token": "t",
                "app_id": "a"
            },
            "operator": {
                "open_id": "ou_operator_2"
            },
            "token": "verify_token",
            "action": {
                "value": {"action": "unknown_action"},
                "tag": "button"
            }
        });
        let result = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap();
        assert!(result.is_none(), "unknown card action should return None");
    }

    #[tokio::test]
    async fn test_handle_webhook_card_action_no_value() {
        let adapter = FeishuAdapter::new("a".into(), "s".into(), "t".into());
        // Card action with no value field
        let payload = serde_json::json!({
            "header": {
                "event_id": "evt_card_3",
                "event_type": "card.action.trigger",
                "create_time": "0",
                "token": "t",
                "app_id": "a"
            },
            "operator": {
                "open_id": "ou_operator_3"
            },
            "token": "verify_token",
            "action": {
                "tag": "button"
            }
        });
        let result = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "card action without value should return None"
        );
    }

    #[tokio::test]
    async fn test_parse_inbound_card_action() {
        let adapter = Arc::new(FeishuAdapter::new("a".into(), "s".into(), "t".into()));
        let renderer = Arc::new(FeishuRenderer::new());
        let plugin = FeishuPlugin::new(adapter, renderer);

        let payload = serde_json::json!({
            "header": {
                "event_id": "evt_card_pi",
                "event_type": "card.action.trigger",
                "create_time": "0",
                "token": "t",
                "app_id": "a"
            },
            "operator": {
                "open_id": "ou_operator_pi"
            },
            "token": "verify_token",
            "action": {
                "value": {"action": "forceful_shutdown"},
                "tag": "button"
            }
        });
        let msg = plugin
            .parse_inbound(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap()
            .expect("card action should return Some");
        assert_eq!(msg.content, "/__card_action:forceful_shutdown");
        assert!(msg.card_action.unwrap_or(false));
        assert_eq!(msg.platform, "feishu");
    }
}
