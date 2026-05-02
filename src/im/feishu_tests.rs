#[cfg(test)]
mod tests {
    use super::*;

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
            "header": {"event_id":"evt_1","event_type":"im.message.receive_v1","create_time":"0","token":"t","app_id":"a"},
            "event": {"sender":{"sender_id":{"open_id":"ou_abc"},"sender_type":"user"},"content":"{\"text\":\"hello\"}","chat_id":"oc_x","message_type":"text"}
        });
        let msg = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap();
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
            "schema":"2.0","header":{"event_id":"e2","event_type":"x","create_time":"0","token":"t","app_id":"a"},
            "event":{"sender":{"sender_id":{"open_id":"ou_x"},"sender_type":"user"},"content":"{\"other\":\"data\"}","chat_id":"oc_y","message_type":"text"}
        });
        let msg = adapter
            .handle_webhook(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap();
        assert_eq!(msg.content, "");
        assert_eq!(msg.metadata.get("account_id"), Some(&"a".to_string()));
    }

    #[test]
    fn test_build_card_body() {
        let card = RichCard {
            card_id: None,
            title: "T".into(),
            elements: vec![],
            header: None,
        };
        assert!(FeishuAdapter::build_card_body(&card).is_ok());
    }

    #[tokio::test]
    async fn test_error_cases() {
        let a = FeishuAdapter::new("bad".into(), "bad".into(), "t".into());
        assert!(a.fetch_tenant_token().await.is_err());
        let card = RichCard {
            card_id: None,
            title: "T".into(),
            elements: vec![],
            header: None,
        };
        assert!(a.send_card("ou", &card).await.is_err());
        let msg = Message {
            id: "1".into(),
            from: "a".into(),
            to: "b".into(),
            content: "hi".into(),
            channel: "feishu".into(),
            timestamp: 0,
            metadata: HashMap::new(),
        };
        assert!(a.send_message(&msg).await.is_err());
    }

    #[tokio::test]
    async fn test_update_message_error() {
        let adapter = FeishuAdapter::new("bad".into(), "bad".into(), "t".into());
        assert!(adapter
            .update_message("om_1", &serde_json::json!({}))
            .await
            .is_err());
    }
}
