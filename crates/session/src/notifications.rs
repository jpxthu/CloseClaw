//! System notification text constants.
//!
//! Per design doc §系统通知接口: notification content is owned by the
//! calling module (Session), not Gateway. Gateway provides the delivery
//! interface (`send_outbound_simplified`).

/// Notification text shown when a message is queued because the session is busy.
pub const QUEUE_NOTIFICATION_TEXT: &str = "⏳ 正在排队...";

/// Default notification text shown when an archived session is being restored.
pub const RESTORE_NOTIFICATION_DEFAULT_TEXT: &str = "正在恢复会话...";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_constants_have_expected_values() {
        assert_eq!(QUEUE_NOTIFICATION_TEXT, "⏳ 正在排队...");
        assert_eq!(RESTORE_NOTIFICATION_DEFAULT_TEXT, "正在恢复会话...");
    }
}
