//! Shared helpers for approval-flow routing in built-in tools.

use serde_json::{Map, Value};

use crate::ToolResult;

/// Build the standard `approval_pending` response payload returned when a
/// denial is successfully enqueued into the approval queue.
pub(crate) fn build_approval_pending(request_id: String) -> Value {
    let mut m = Map::new();
    m.insert("status".into(), "approval_pending".into());
    m.insert("request_id".into(), request_id.into());
    m.insert("message".into(), "Operation pending owner approval".into());
    Value::Object(m)
}

/// Build the full approval-pending [`ToolResult`] returned when a denial
/// is successfully enqueued into the approval queue (payload from
/// [`build_approval_pending`], no messages, no context modifier).
pub(crate) fn approval_pending(request_id: String) -> ToolResult {
    ToolResult {
        data: build_approval_pending(request_id),
        new_messages: vec![],
        context_modifier: None,
    }
}
