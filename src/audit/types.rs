//! Audit event types and data structures

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Audit event types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event_type")]
pub enum AuditEventType {
    PermissionCheck,
    AgentStart,
    AgentStop,
    AgentError,
    ConfigReload,
    RuleReload,
}

/// Result of an audited operation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuditResult {
    Allow,
    Deny,
    Error,
}

/// An audit event record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Event timestamp
    pub timestamp: DateTime<Local>,
    /// Type of event
    pub event_type: AuditEventType,
    /// Detailed context as JSON
    pub details: serde_json::Value,
    /// Result of the operation
    pub result: AuditResult,
}

impl AuditEvent {
    /// Create a new audit event
    pub fn new(
        event_type: AuditEventType,
        details: serde_json::Value,
        result: AuditResult,
    ) -> Self {
        Self {
            timestamp: Local::now(),
            event_type,
            details,
            result,
        }
    }

    /// Serialize to a JSON line (one JSON object per line)
    pub fn serialize_to_json(&self) -> String {
        serde_json::to_string(self).expect("audit event should serialize to JSON")
    }
}

/// Audit event builder for convenient construction
pub struct AuditEventBuilder {
    event_type: AuditEventType,
    details: serde_json::Value,
    result: AuditResult,
}

impl AuditEventBuilder {
    pub fn new(event_type: AuditEventType) -> Self {
        Self {
            event_type,
            details: json!({}),
            result: AuditResult::Allow,
        }
    }

    pub fn details(mut self, details: serde_json::Value) -> Self {
        self.details = details;
        self
    }

    pub fn result(mut self, result: AuditResult) -> Self {
        self.result = result;
        self
    }

    pub fn build(self) -> AuditEvent {
        AuditEvent::new(self.event_type, self.details, self.result)
    }
}
