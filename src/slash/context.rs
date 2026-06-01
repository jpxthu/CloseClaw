/// Execution context for a slash command invocation.
pub struct SlashContext {
    pub sender_id: String,
    pub session_id: String,
    pub channel: String,
}
