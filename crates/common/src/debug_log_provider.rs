use closeclaw_debug_log::DebugLog;

/// Trait for providing access to a [`DebugLog`] instance.
///
/// Implementations return `None` when debug logging is not configured
/// or not available, allowing callers to gracefully skip logging.
pub trait DebugLogProvider {
    /// Returns a reference to the active [`DebugLog`], or `None` if
    /// debug logging is not configured.
    fn debug_log(&self) -> Option<&DebugLog>;
}
