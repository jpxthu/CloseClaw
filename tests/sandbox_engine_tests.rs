//! Unit tests for sandbox engine environment variable branching logic.
//!
//! Tests `detect_engine_mode_inner()` — the pure detection function that
//! determines whether the process should enter engine mode based on
//! `SANDBOX_ENGINE` and `SANDBOX_IPC_PATH` values.
//!
//! These tests do NOT start any real subprocess; they only verify the
//! decision logic. `std::env::set_var` / `remove_var` are NOT used.

use closeclaw::sandbox_engine::detect_engine_mode_inner;

// --- Normal path: engine mode activated ---

#[test]
fn test_engine_mode_active_with_valid_ipc_path() {
    let result = detect_engine_mode_inner(Some("1"), Some("/tmp/test.sock"));
    let inner = result.expect("should return Some for engine mode");
    let (ipc_path, _rules) = inner.expect("should be Ok with valid path");
    assert_eq!(ipc_path, std::path::PathBuf::from("/tmp/test.sock"));
}

// --- Boundary: SANDBOX_ENGINE not set ---

#[test]
fn test_no_engine_flag_returns_none() {
    let result = detect_engine_mode_inner(None, Some("/tmp/test.sock"));
    assert!(result.is_none(), "None engine flag should yield None (normal CLI)");
}

#[test]
fn test_engine_flag_empty_string_returns_none() {
    let result = detect_engine_mode_inner(Some(""), Some("/tmp/test.sock"));
    assert!(
        result.is_none(),
        "Empty engine flag should yield None (normal CLI)"
    );
}

#[test]
fn test_engine_flag_not_one_returns_none() {
    let result = detect_engine_mode_inner(Some("0"), Some("/tmp/test.sock"));
    assert!(
        result.is_none(),
        "Engine flag '0' should yield None (normal CLI)"
    );
}

// --- Error path: engine mode requested but IPC path missing ---

#[test]
fn test_engine_active_without_ipc_path_returns_error() {
    let result = detect_engine_mode_inner(Some("1"), None);
    let err = result
        .expect("should return Some even on misconfiguration")
        .expect_err("should be Err when IPC path is missing");
    assert!(
        err.to_string().contains("SANDBOX_IPC_PATH"),
        "error message should mention SANDBOX_IPC_PATH, got: {}",
        err
    );
}

#[test]
fn test_engine_active_with_empty_ipc_path_returns_error() {
    let result = detect_engine_mode_inner(Some("1"), Some(""));
    let err = result
        .expect("should return Some even on misconfiguration")
        .expect_err("should be Err when IPC path is empty");
    assert!(
        err.to_string().contains("SANDBOX_IPC_PATH"),
        "error message should mention SANDBOX_IPC_PATH, got: {}",
        err
    );
}

// --- Edge: both unset ---

#[test]
fn test_both_unset_returns_none() {
    let result = detect_engine_mode_inner(None, None);
    assert!(
        result.is_none(),
        "Both unset should yield None (normal CLI flow)"
    );
}

// --- RuleSet default check ---

#[test]
fn test_engine_mode_returns_default_ruleset() {
    let result = detect_engine_mode_inner(Some("1"), Some("/tmp/test.sock"));
    let (_ipc_path, rules) = result
        .expect("should return Some")
        .expect("should be Ok");
    // Default RuleSet should have no explicit rules
    assert!(
        rules.rules.is_empty(),
        "default RuleSet should have no rules"
    );
}
