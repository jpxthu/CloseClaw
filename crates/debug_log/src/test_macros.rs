//! Shared cross-crate test macros for debug-log wrapper modules.
//!
//! The business crates (`llm`, `session`, `slash`, `tools`,
//! `permission`, `processor_chain`, `tasks`) previously embedded nearly
//! identical debug-log wrapper unit tests in their `src/debug_log.rs`.
//! These macros expand the same test functions — identical names and
//! verbatim-equivalent assertions — so each crate keeps a single macro
//! invocation instead of duplicated inline bodies.
//!
//! Invoke each macro at most once per module: the expanded test
//! functions carry fixed names, so a second expansion in the same
//! module would collide.

/// Expand the three wrapper-layer debug-log tests.
///
/// For crates exposing a thin emit wrapper plus per-crate type aliases
/// (see `crates/*/src/debug_log.rs`). Arguments: the wrapper function
/// ident, the context type alias ident, the params type alias ident,
/// and the fixed `source_module` string literal.
///
/// Invoke inside a `#[cfg(test)]` module where the wrapper items are in
/// scope (e.g. via `use super::*`). Otherwise the expansion is
/// self-contained: it only references `closeclaw_debug_log` and
/// `serde_json` by path, so business crates need no extra imports.
#[macro_export]
macro_rules! debug_log_wrapper_tests {
    ($emit_fn:ident, $ctx_ty:ident, $params_ty:ident, $source_module:literal) => {
        $crate::debug_log_shared_tests!(all; $emit_fn, $ctx_ty, $params_ty, $source_module);
    };
}

/// Expand the two core-variant debug-log tests (no wrapper layer).
///
/// For crates calling `emit_event` directly with
/// the native context/params types (currently `tasks`). The only
/// argument is the fixed `source_module` string literal; the expansion
/// is fully self-contained.
#[macro_export]
macro_rules! debug_log_core_tests {
    ($source_module:literal) => {
        $crate::debug_log_shared_tests!(
            shared;
            $crate::emit_event,
            $crate::DebugLogContext,
            $crate::EmitEventParams,
            $source_module
        );
    };
}

/// Delegation target holding every wrapper-layer test body exactly
/// once, so the public variants share verbatim-equivalent assertions.
///
/// Internal implementation detail; not part of the public API.
#[doc(hidden)]
#[macro_export]
macro_rules! debug_log_shared_tests {
    // Full wrapper-layer expansion: the two tests shared with the core
    // variant plus the wrapper-only empty-trace-id test.
    (all; $emit_fn:path, $ctx_ty:path, $params_ty:path, $source_module:literal) => {
        $crate::debug_log_shared_tests!(shared; $emit_fn, $ctx_ty, $params_ty, $source_module);

        #[test]
        fn test_empty_trace_id_is_noop() {
            // Hygienic alias: `:path` fragments are opaque and cannot
            // head a struct literal, so bind the params type to a plain
            // ident (`Params` stays macro-internal, no user collision).
            use $params_ty as Params;
            let ctx = <$ctx_ty>::new(None, "", None);
            $emit_fn(Params {
                ctx,
                level: $crate::LogLevel::Info,
                source_module: $source_module,
                event_type: "test.event",
                payload: serde_json::json!({}),
                parent: None,
            });
        }
    };
    // Tests shared by both public variants, written exactly once.
    (shared; $emit_fn:path, $ctx_ty:path, $params_ty:path, $source_module:literal) => {
        #[test]
        fn test_none_debug_log_is_noop() {
            // Hygienic alias: `:path` fragments are opaque and cannot
            // head a struct literal, so bind the params type to a plain
            // ident (`Params` stays macro-internal, no user collision).
            use $params_ty as Params;
            let ctx = <$ctx_ty>::new(None, "trace-123", None);
            $emit_fn(Params {
                ctx,
                level: $crate::LogLevel::Info,
                source_module: $source_module,
                event_type: "test.event",
                payload: serde_json::json!({}),
                parent: None,
            });
        }

        #[test]
        fn test_debug_log_context_new_fields() {
            let ctx = <$ctx_ty>::new(None, "tid", Some("skey"));
            assert_eq!(ctx.trace_id, "tid");
            assert_eq!(ctx.session_key, Some("skey"));
            assert!(ctx.debug_log.is_none());
        }
    };
}

#[cfg(test)]
#[path = "test_macros_tests.rs"]
mod tests;
