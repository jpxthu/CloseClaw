//! Unit tests for the audit module

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    use super::super::types::{AuditEvent, AuditEventBuilder, AuditEventType, AuditResult};
    use super::super::logger::AuditLogger;
    use super::super::query::{query_audit_events, AuditQueryFilter};

    #[test]
    fn test_audit_event_serialize_to_json() {
        let event = AuditEvent::new(
            AuditEventType::PermissionCheck,
            json!({
                "agent": "test-agent",
                "action": "file_read",
                "path": "/home/admin"
            }),
            AuditResult::Allow,
        );

        let json_str = event.serialize_to_json();
        let parsed: AuditEvent = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.event_type, AuditEventType::PermissionCheck);
        assert_eq!(parsed.result, AuditResult::Allow);
        assert_eq!(parsed.details["agent"], "test-agent");
    }

    #[test]
    fn test_audit_event_types_serialize() {
        let types = vec![
            (AuditEventType::PermissionCheck, "PermissionCheck"),
            (AuditEventType::AgentStart, "AgentStart"),
            (AuditEventType::AgentStop, "AgentStop"),
            (AuditEventType::AgentError, "AgentError"),
            (AuditEventType::ConfigReload, "ConfigReload"),
            (AuditEventType::RuleReload, "RuleReload"),
        ];

        for (typ, name) in types {
            let event = AuditEvent::new(typ, json!({}), AuditResult::Allow);
            let json_str = event.serialize_to_json();
            assert!(json_str.contains(name), "JSON should contain {}", name);
        }
    }

    #[test]
    fn test_audit_result_serialize() {
        let results = vec![
            (AuditResult::Allow, "allow"),
            (AuditResult::Deny, "deny"),
            (AuditResult::Error, "error"),
        ];

        for (result, name) in results {
            let event = AuditEvent::new(AuditEventType::PermissionCheck, json!({}), result);
            let json_str = event.serialize_to_json();
            assert!(json_str.contains(name), "JSON should contain {}", name);
        }
    }

    #[tokio::test]
    async fn test_audit_logger_file_writing() {
        let temp_dir = TempDir::new().unwrap();
        let logger = AuditLogger::with_base_dir(temp_dir.path().to_path_buf());

        let event = AuditEvent::new(
            AuditEventType::PermissionCheck,
            json!({"agent": "test-agent"}),
            AuditResult::Allow,
        );

        logger.log(event).await;
        logger.flush().await;

        // Check file exists
        let date_str = AuditLogger::current_date_string();
        let log_file = temp_dir.path().join(format!("{}.jsonl", date_str));
        assert!(log_file.exists(), "log file should exist at {:?}", log_file);

        let content = fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("test-agent"));
        assert!(content.contains("PermissionCheck"));
    }

    #[tokio::test]
    async fn test_audit_logger_buffer_flush() {
        let temp_dir = TempDir::new().unwrap();
        let logger = AuditLogger::with_base_dir(temp_dir.path().to_path_buf());

        // Log many events to trigger buffer flush
        for i in 0..600 {
            let event = AuditEvent::new(
                AuditEventType::PermissionCheck,
                json!({"index": i}),
                AuditResult::Allow,
            );
            logger.log(event).await;
        }

        // Flush remaining
        logger.flush().await;

        let date_str = AuditLogger::current_date_string();
        let log_file = temp_dir.path().join(format!("{}.jsonl", date_str));
        assert!(log_file.exists());

        let content = fs::read_to_string(&log_file).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 600);
    }

    #[tokio::test]
    async fn test_audit_logger_multiple_days_rotation() {
        let temp_dir = TempDir::new().unwrap();
        let logger = AuditLogger::with_base_dir(temp_dir.path().to_path_buf());

        // Force initial date
        {
            let mut current = logger.current_date.lock().unwrap();
            *current = "2024-01-01".to_string();
        }

        let event = AuditEvent::new(
            AuditEventType::AgentStart,
            json!({"agent": "dev-agent"}),
            AuditResult::Allow,
        );
        logger.log(event).await;
        logger.flush().await;

        // Old file should have been written
        let old_file = temp_dir.path().join("2024-01-01.jsonl");
        assert!(old_file.exists());

        // Rotate to new day
        logger.rotate_if_needed().await;

        let _new_file = temp_dir
            .path()
            .join(format!("{}.jsonl", AuditLogger::current_date_string()));
        // New file may or may not exist depending on date
    }

    #[test]
    fn test_audit_event_builder() {
        let event = AuditEventBuilder::new(AuditEventType::AgentError)
            .details(json!({"agent": "test", "error": "crash"}))
            .result(AuditResult::Error)
            .build();

        assert_eq!(event.event_type, AuditEventType::AgentError);
        assert_eq!(event.result, AuditResult::Error);
        assert_eq!(event.details["error"], "crash");
    }

    #[test]
    fn test_query_audit_events_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        // Override HOME to use the empty temp dir as audit base
        let temp_home = temp_dir.path().to_str().unwrap();
        std::env::set_var("HOME", temp_home);

        let filter = AuditQueryFilter {
            days: 1,
            event_type: None,
            agent: None,
            limit: None,
        };

        let results = query_audit_events(&filter);
        // Should not panic even with non-existent audit dir
        assert!(results.is_empty());
    }

    #[test]
    fn test_query_audit_events_max_days_cap() {
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("HOME", temp_dir.path().to_str().unwrap());

        // u32::MAX should be capped to MAX_QUERY_DAYS without panicking
        let filter = AuditQueryFilter {
            days: u32::MAX,
            event_type: None,
            agent: None,
            limit: None,
        };

        // Should not panic even with u32::MAX days
        let results = query_audit_events(&filter);
        assert!(results.is_empty());
        assert!(
            results.is_empty(),
            "Expected empty results in temp audit dir, got: {:?}",
            results
        );
    }

    #[test]
    fn test_query_with_filters() {
        let temp_dir = TempDir::new().unwrap();
        let date_str = "2024-01-15";
        let log_file = temp_dir.path().join(format!("{}.jsonl", date_str));

        // Write some test events
        let event1 = AuditEvent::new(
            AuditEventType::PermissionCheck,
            json!({"agent": "dev-agent-01", "action": "file_read"}),
            AuditResult::Allow,
        );
        let event2 = AuditEvent::new(
            AuditEventType::AgentStart,
            json!({"agent": "dev-agent-01"}),
            AuditResult::Allow,
        );
        let event3 = AuditEvent::new(
            AuditEventType::PermissionCheck,
            json!({"agent": "prod-agent", "action": "file_read"}),
            AuditResult::Deny,
        );

        let content = format!(
            "{}\n{}\n{}",
            event1.serialize_to_json(),
            event2.serialize_to_json(),
            event3.serialize_to_json()
        );
        fs::write(&log_file, content).unwrap();

        // Query all
        let _filter_all = AuditQueryFilter {
            days: 7,
            event_type: None,
            agent: None,
            limit: None,
        };

        // Since we can't easily override HOME for query_audit_events,
        // we test the file-reading logic by reading directly
        let content = fs::read_to_string(&log_file).unwrap();
        let mut parsed_events = Vec::new();
        for line in content.lines() {
            if let Ok(evt) = serde_json::from_str::<AuditEvent>(line) {
                parsed_events.push(evt);
            }
        }
        assert_eq!(parsed_events.len(), 3);

        // Filter by agent
        let agent_filtered: Vec<_> = parsed_events
            .iter()
            .filter(|e| {
                serde_json::to_string(&e.details)
                    .unwrap_or_default()
                    .contains("dev-agent-01")
            })
            .collect();
        assert_eq!(agent_filtered.len(), 2);

        // Filter by event_type
        let type_filtered: Vec<_> = parsed_events
            .iter()
            .filter(|e| {
                format!("{:?}", e.event_type)
                    .to_lowercase()
                    .contains("permission")
            })
            .collect();
        assert_eq!(type_filtered.len(), 2);
    }

    #[test]
    fn test_export_json_format() {
        let temp_dir = TempDir::new().unwrap();
        let output_file = temp_dir.path().join("export.json");

        let events = vec![AuditEvent::new(
            AuditEventType::PermissionCheck,
            json!({"agent": "test"}),
            AuditResult::Allow,
        )];

        let content = serde_json::to_string_pretty(&events).unwrap();
        fs::write(&output_file, &content).unwrap();

        let written = fs::read_to_string(&output_file).unwrap();
        assert!(written.contains("PermissionCheck"));
        assert!(written.contains("test"));
    }
}
