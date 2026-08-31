use chrono::Utc;

/// Generate a unique trace ID for non-message events.
///
/// Format: `{module_name}_{timestamp_hex}_{random_hex}`
///
/// This is used by modules that trigger events outside the message chain
/// (e.g. scheduled tasks, background jobs) to produce independently
/// traceable IDs.
///
/// The `module_name` identifies the originating module (e.g. `"tasks"`,
/// `"daemon"`). The timestamp is the current UTC time in milliseconds
/// as hex. The random component is a UUID v4 with hyphens removed.
pub fn generate_trace_id(module_name: &str) -> String {
    let timestamp_hex = format!("{:x}", Utc::now().timestamp_millis());
    let random_hex = uuid::Uuid::new_v4().simple().to_string();
    format!("{module_name}_{timestamp_hex}_{random_hex}")
}

#[cfg(test)]
#[path = "trace_id_tests.rs"]
mod trace_id_tests;
