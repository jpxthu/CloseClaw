#[cfg(test)]
mod tests {
    use crate::im::NormalizedMessage;

    #[test]
    fn test_roundtrip_with_all_fields() {
        let msg = NormalizedMessage {
            platform: "feishu".to_string(),
            sender_id: "ou_abc123".to_string(),
            peer_id: "oc_group456".to_string(),
            content: "Hello, world!".to_string(),
            timestamp: 1700000000,
            thread_id: Some("omt_thread789".to_string()),
            account_id: Some("tenant_001".to_string()),
        };

        let json = serde_json::to_string(&msg).expect("serialization failed");
        let deserialized: NormalizedMessage =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.platform, "feishu");
        assert_eq!(deserialized.sender_id, "ou_abc123");
        assert_eq!(deserialized.peer_id, "oc_group456");
        assert_eq!(deserialized.content, "Hello, world!");
        assert_eq!(deserialized.timestamp, 1700000000);
        assert_eq!(deserialized.thread_id.as_deref(), Some("omt_thread789"));
        assert_eq!(deserialized.account_id.as_deref(), Some("tenant_001"));
    }

    #[test]
    fn test_roundtrip_with_none_optional_fields() {
        let msg = NormalizedMessage {
            platform: "discord".to_string(),
            sender_id: "user123".to_string(),
            peer_id: "dm456".to_string(),
            content: "Direct message".to_string(),
            timestamp: 1700000000,
            thread_id: None,
            account_id: None,
        };

        let json = serde_json::to_string(&msg).expect("serialization failed");
        let deserialized: NormalizedMessage =
            serde_json::from_str(&json).expect("deserialization failed");

        assert_eq!(deserialized.platform, "discord");
        assert_eq!(deserialized.sender_id, "user123");
        assert_eq!(deserialized.peer_id, "dm456");
        assert_eq!(deserialized.content, "Direct message");
        assert_eq!(deserialized.timestamp, 1700000000);
        assert!(deserialized.thread_id.is_none());
        assert!(deserialized.account_id.is_none());
    }

    #[test]
    fn test_json_field_names_are_correct() {
        let msg = NormalizedMessage {
            platform: "test".to_string(),
            sender_id: "s".to_string(),
            peer_id: "p".to_string(),
            content: "c".to_string(),
            timestamp: 42,
            thread_id: None,
            account_id: None,
        };

        let json = serde_json::to_value(&msg).expect("serialization to value failed");
        let obj = json.as_object().expect("expected JSON object");

        assert!(obj.contains_key("platform"));
        assert!(obj.contains_key("sender_id"));
        assert!(obj.contains_key("peer_id"));
        assert!(obj.contains_key("content"));
        assert!(obj.contains_key("timestamp"));
        assert!(obj.contains_key("thread_id"));
        assert!(obj.contains_key("account_id"));
        assert_eq!(obj.len(), 7);
    }
}
