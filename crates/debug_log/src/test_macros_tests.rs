//! Unit tests for the shared debug-log test macros.
//!
//! Both macro variants are expanded on a local wrapper mimicking a
//! business crate's `debug_log` module (type aliases + thin emit
//! wrapper), verifying macro hygiene and assertion correctness in one
//! place. Expanding both variants in the same file also proves they
//! coexist without collisions (each is expanded exactly once per
//! module).

/// Local wrapper mimicking a business crate's debug-log module.
mod wrapper {
    /// Type alias for the common `DebugLogContext`.
    pub type WrapperDebugLogContext<'a> = crate::DebugLogContext<'a>;

    /// Type alias for the common `EmitEventParams`.
    pub type WrapperEmitEventParams<'a> = crate::EmitEventParams<'a>;

    /// Thin wrapper over `emit_event`, mirroring the business crates.
    pub fn emit_wrapper_event(params: WrapperEmitEventParams<'_>) {
        crate::emit_event(params);
    }
}

mod wrapper_tests {
    use super::wrapper::*;

    crate::debug_log_wrapper_tests!(
        emit_wrapper_event,
        WrapperDebugLogContext,
        WrapperEmitEventParams,
        "wrapper"
    );
}

mod core_tests {
    crate::debug_log_core_tests!("test_macros");
}
