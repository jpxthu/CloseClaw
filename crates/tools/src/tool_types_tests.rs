//! Tests for ToolSummary and ToolError (migrated from common/tool_trait_tests).

use super::{ToolError, ToolSummary};

// =========================================================================
// ToolSummary
// =========================================================================

#[test]
fn test_tool_summary_fields() {
    let summary = ToolSummary {
        name: "Read".into(),
        group: "file_ops".into(),
        summary: "Read file contents".into(),
        is_deferred: false,
    };
    assert_eq!(summary.name, "Read");
    assert_eq!(summary.group, "file_ops");
    assert_eq!(summary.summary, "Read file contents");
    assert!(!summary.is_deferred);
}

#[test]
fn test_tool_summary_clone() {
    let summary = ToolSummary {
        name: "Write".into(),
        group: "file_ops".into(),
        summary: "Write file contents".into(),
        is_deferred: true,
    };
    let cloned = summary.clone();
    assert_eq!(cloned.name, "Write");
    assert!(cloned.is_deferred);
}

#[test]
fn test_tool_summary_debug() {
    let summary = ToolSummary {
        name: "Bash".into(),
        group: "bash".into(),
        summary: "Run commands".into(),
        is_deferred: false,
    };
    let debug = format!("{:?}", summary);
    assert!(debug.contains("ToolSummary"));
    assert!(debug.contains("Bash"));
}

// =========================================================================
// ToolError
// =========================================================================

#[test]
fn test_tool_error_not_found_display() {
    let err = ToolError::NotFound("Bash".into());
    assert_eq!(format!("{}", err), "tool not found: Bash");
}

#[test]
fn test_tool_error_already_registered_display() {
    let err = ToolError::AlreadyRegistered("Read".into());
    assert_eq!(format!("{}", err), "tool `Read` already registered");
}
