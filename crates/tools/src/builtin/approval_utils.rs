//! Shared helpers for approval-flow routing in built-in tools.

use serde_json::{Map, Value};

/// Build the standard `approval_pending` response payload returned when a
/// denial is successfully enqueued into the approval queue.
pub(crate) fn build_approval_pending(request_id: String) -> Value {
    let mut m = Map::new();
    m.insert("status".into(), "approval_pending".into());
    m.insert("request_id".into(), request_id.into());
    m.insert("message".into(), "Operation pending owner approval".into());
    Value::Object(m)
}
