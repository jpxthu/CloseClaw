//! Inbox module tests

use super::types::{InboxConfig, InboxMessage, MessageStatus, MessageType};

fn test_config() -> InboxConfig {
    InboxConfig {
        poll_interval_secs: 5,
        max_retry: 3,
        base_delay_ms: 1000,
        max_delay_ms: 60000,
        jitter_ms: 500,
        timeout_ms: 10000,
        acked_ttl_days: 7,
        dead_letter_ttl_days: 30,
        alert_webhook: None,
    }
}

#[test]
fn test_message_creation() {
    let task_msg = InboxMessage::new(
        "parent-1".to_string(),
        "child-1".to_string(),
        MessageType::Task,
        serde_json::json!({"data": "test"}),
    );

    assert_eq!(task_msg.status, MessageStatus::Pending);
    assert_eq!(task_msg.retry_count, 0);
    assert!(task_msg.should_persist());
    assert!(task_msg.should_retry()); // Task should retry

    let heartbeat_msg = InboxMessage::new(
        "parent-1".to_string(),
        "child-1".to_string(),
        MessageType::Heartbeat,
        serde_json::json!({}),
    );
    assert!(!heartbeat_msg.should_persist()); // Heartbeat should not persist
    assert!(!heartbeat_msg.should_retry()); // Heartbeat should not retry
}

#[test]
fn test_message_ack() {
    let mut msg = InboxMessage::new(
        "parent-1".to_string(),
        "child-1".to_string(),
        MessageType::Task,
        serde_json::json!({}),
    );
    msg.ack();
    assert_eq!(msg.status, MessageStatus::Acked);
    assert!(msg.acked_at.is_some());
}

#[test]
fn test_message_dead_letter() {
    let mut msg = InboxMessage::new(
        "parent-1".to_string(),
        "child-1".to_string(),
        MessageType::Task,
        serde_json::json!({}),
    );
    msg.dead_letter("max_retries_exceeded");
    assert_eq!(msg.status, MessageStatus::DeadLetter);
    assert!(msg.dead_letter_at.is_some());
    assert_eq!(msg.last_error, Some("max_retries_exceeded".to_string()));
}

#[test]
fn test_message_should_persist() {
    let task_msg = InboxMessage::new(
        "from".to_string(),
        "to".to_string(),
        MessageType::Task,
        serde_json::json!({}),
    );
    let heartbeat_msg = InboxMessage::new(
        "from".to_string(),
        "to".to_string(),
        MessageType::Heartbeat,
        serde_json::json!({}),
    );

    assert!(task_msg.should_persist());
    assert!(!heartbeat_msg.should_persist());
}

#[test]
fn test_exponential_backoff() {
    let mut msg = InboxMessage::new(
        "from".to_string(),
        "to".to_string(),
        MessageType::Task,
        serde_json::json!({}),
    );
    msg.max_retry = 3;
    let config = test_config();

    // First retry
    let next = msg.calculate_next_retry(&config);
    assert!(next.is_some());
    // Delay should be around 1000ms +/- 500ms (so 500-1500ms)
    if let Some(t) = next {
        let delay = (t - msg.created_at).num_milliseconds();
        assert!(
            delay >= 500 && delay <= 1500,
            "Expected 500-1500ms, got {}",
            delay
        );
    }

    // Increment retry count and check again
    msg.retry_count = 1;
    let next = msg.calculate_next_retry(&config);
    assert!(next.is_some());
    if let Some(t) = next {
        let delay = (t - msg.created_at).num_milliseconds();
        // Should be around 2000ms +/- 500ms (so 1500-2500ms)
        assert!(
            delay >= 1500 && delay <= 2500,
            "Expected 1500-2500ms, got {}",
            delay
        );
    }
}

#[test]
fn test_max_retry_exceeded() {
    let mut msg = InboxMessage::new(
        "from".to_string(),
        "to".to_string(),
        MessageType::Task,
        serde_json::json!({}),
    );
    msg.retry_count = 3; // Equal to max_retry
    msg.max_retry = 3;
    let config = test_config();

    let next = msg.calculate_next_retry(&config);
    assert!(next.is_none()); // No more retries
}
