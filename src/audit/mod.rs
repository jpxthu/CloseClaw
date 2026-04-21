//! Audit logging system for CloseClaw
//!
//! Records permission checks, agent operations, and errors to persistent JSONL files.

pub mod logger;
pub mod query;
pub mod tests;
pub mod types;

pub use logger::AuditLogger;
pub use query::{export_audit_events, query_audit_events, AuditQueryFilter, MAX_QUERY_DAYS};
pub use types::{AuditEvent, AuditEventBuilder, AuditEventType, AuditResult};
