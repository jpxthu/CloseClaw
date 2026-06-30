/// Execution context for a slash command invocation.
pub struct SlashContext {
    /// The slash command name (without the leading `/`).
    ///
    /// For multi-command handlers (e.g. `WorkdirHandler` handling `cd`,
    /// `pwd`, `git`) this lets `handle()` branch on the invoked subcommand.
    pub command: String,
    pub sender_id: String,
    pub session_id: String,
    pub channel: String,
}
