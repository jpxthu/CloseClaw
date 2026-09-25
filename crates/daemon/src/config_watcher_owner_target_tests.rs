//! Whitespace-dimension behavior tests for `parse_owner_target`
//! (issue #3251): leading/trailing whitespace around the whole
//! `owner_display` value and around each `platform:chat_id` segment is
//! trimmed (normalization), while a segment that is empty *after*
//! trimming is rejected exactly like the pre-existing empty-segment
//! cases (warn + `None` → owner notification skipped, same semantics as
//! "not configured").
//!
//! Added as a new sibling test module (nothing was split out of
//! `config_reload_tests.rs`) so that file stays within the 1000-line
//! limit (CONTRIBUTING.md hard cap) — same precedent as
//! `config_watcher_select_tests.rs` (issue #3220).
//!
//! Dimensions covered here (behavior, not implementation):
//! - normalization (boundary): whitespace around the colon or at the
//!   edges of the value — including the ideographic space U+3000, which
//!   `str::trim` covers — yields the trimmed segments
//! - rejection (error path): a segment that is empty after trimming
//!   (`"feishu: "`, `" :oc"`, `" : "`) and a pure-whitespace value
//!   without a colon are all rejected with `None`
//! - regression (happy path): a value without any whitespace keeps the
//!   exact pre-fix return contract
//!
//! Non-whitespace dimensions (missing `commands` / missing
//! `owner_display` / no colon / pre-fix empty segments) are already
//! covered by the six `test_parse_owner_target_*` cases in
//! `config_reload_tests.rs` and deliberately not duplicated here.

use super::*;
use crate::test_helpers::load_system_config_manager;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// parse_owner_target whitespace coverage matrix — case × input × branch hit
// (branch lines refer to config_watcher.rs as of the issue #3251 fix; the
// six pre-whitespace cases and their matrix live in
// `config_reload_tests.rs` and are maintained there. The verbatim-equal second
// definition in `config_reload/reload.rs` inherits this behavior via the
// synchronized fix; direct tests for it are issue #3241 scope.)
//
//   test_owner_target_whitespace_after_colon_normalized    "feishu: oc"
//     → :292 Some (segments trimmed)
//   test_owner_target_whitespace_before_colon_normalized   " feishu:oc"
//     → :292 Some
//   test_owner_target_whitespace_platform_side_normalized  "feishu :oc"
//     → :292 Some
//   test_owner_target_ideographic_space_normalized         "　feishu:oc　"
//     → :292 Some (trim covers U+3000)
//   test_owner_target_whitespace_chat_id_rejected          "feishu: "
//     → :285 parts[1].trim().is_empty() → :290 None
//   test_owner_target_whitespace_platform_rejected         " :oc"
//     → :285 parts[0].trim().is_empty() → :290 None
//   test_owner_target_all_whitespace_pair_rejected         " : "
//     → :285 parts[0].trim().is_empty() → :290 None
//   test_owner_target_pure_whitespace_rejected             "   "
//     → :285 parts.len() != 2 → :290 None
//   test_owner_target_no_whitespace_baseline               "feishu:oc_xxx"
//     → :292 Some (regression baseline)
// ---------------------------------------------------------------------------

/// Shared setup for the whitespace cases: write `system.json` with the
/// per-case `owner_display` payload, load the `System` section into a
/// fresh [`ConfigManager`], then parse the owner target from it.
///
/// Same shape as `parse_owner_target_from` in `config_reload_tests.rs`:
/// the mandatory reload step is checked via the caller-supplied
/// `reload_expect` message; the per-case `assert_eq!` stays in the
/// calling test.
fn parse_owner_target_from(
    system_json: serde_json::Value,
    reload_expect: &str,
) -> Option<(String, String)> {
    let tmp = TempDir::new().unwrap();
    let cm = load_system_config_manager(tmp.path(), system_json, reload_expect);
    parse_owner_target(&cm)
}

/// Normalization: whitespace between the colon and the chat_id is
/// trimmed, so `"feishu: oc"` delivers `Some(("feishu", "oc"))` — the
/// notification must still reach the owner instead of failing at
/// `plugin.send` with a whitespace-only chat_id.
#[test]
fn test_owner_target_whitespace_after_colon_normalized() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "feishu: oc"
            }
        }),
        "reload system.json with padded chat_id succeeds",
    );
    assert_eq!(result, Some(("feishu".to_string(), "oc".to_string())));
}

/// Normalization: leading whitespace before the platform segment is
/// trimmed, so `" feishu:oc"` yields the same clean pair as the
/// no-whitespace happy path.
#[test]
fn test_owner_target_whitespace_before_colon_normalized() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": " feishu:oc"
            }
        }),
        "reload system.json with leading-whitespace owner_display succeeds",
    );
    assert_eq!(result, Some(("feishu".to_string(), "oc".to_string())));
}

/// Normalization: whitespace before the colon is trimmed from the
/// platform segment, so `"feishu :oc"` yields `Some(("feishu", "oc"))`
/// — the segment guards look at the trimmed segments, not the raw split
/// parts.
#[test]
fn test_owner_target_whitespace_platform_side_normalized() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "feishu :oc"
            }
        }),
        "reload system.json with padded platform succeeds",
    );
    assert_eq!(result, Some(("feishu".to_string(), "oc".to_string())));
}

/// Normalization: `str::trim` removes Unicode whitespace, so the
/// ideographic space U+3000 (`　`) around the value is treated like
/// ASCII whitespace — relevant for hand-edited CJK configs.
#[test]
fn test_owner_target_ideographic_space_normalized() {
    // Self-proof before the call: U+3000 really is `char::is_whitespace`,
    // so this case exercises the trim path rather than duplicating the
    // ASCII cases above.
    assert!(
        '\u{3000}'.is_whitespace(),
        "U+3000 must be `char::is_whitespace` for str::trim to strip it"
    );

    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "\u{3000}feishu:oc\u{3000}"
            }
        }),
        "reload system.json with ideographic-space padding succeeds",
    );
    assert_eq!(result, Some(("feishu".to_string(), "oc".to_string())));
}

/// Rejection: a chat_id that is empty *after* trimming (`"feishu: "`)
/// is treated as not configured — this is the silent-notification-loss
/// shape from issue #3251, where the whitespace-only chat_id used to
/// pass the old `is_empty()` guards and fail later at `plugin.send`.
#[test]
fn test_owner_target_whitespace_chat_id_rejected() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "feishu: "
            }
        }),
        "reload system.json with whitespace-only chat_id succeeds",
    );
    assert_eq!(result, None);
}

/// Rejection: a platform segment that is empty after trimming
/// (`" :oc"`) trips the same guard as the pre-fix `":oc_xxx"` case —
/// whitespace does not make a missing platform valid.
#[test]
fn test_owner_target_whitespace_platform_rejected() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": " :oc"
            }
        }),
        "reload system.json with whitespace-only platform succeeds",
    );
    assert_eq!(result, None);
}

/// Rejection: both segments empty after trimming (`" : "`) — neither a
/// platform nor a chat_id survives the trim, so the format guard
/// rejects the value regardless of which operand is checked first.
#[test]
fn test_owner_target_all_whitespace_pair_rejected() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": " : "
            }
        }),
        "reload system.json with all-whitespace pair succeeds",
    );
    assert_eq!(result, None);
}

/// Rejection: a pure-whitespace value without a colon (`"   "`) is
/// rejected by the segment-count arm (`splitn(2, ':')` yields one
/// element) — the no-colon error path must keep firing for
/// whitespace-only input, not only for visible text like
/// `"no-colon-here"` (covered by `config_reload_tests.rs`).
#[test]
fn test_owner_target_pure_whitespace_rejected() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "   "
            }
        }),
        "reload system.json with pure-whitespace owner_display succeeds",
    );
    assert_eq!(result, None);
}

/// Regression: a value without any whitespace keeps the exact pre-fix
/// contract — `"feishu:oc_xxx"` still yields
/// `Some(("feishu", "oc_xxx"))` verbatim, so the normalization added by
/// issue #3251 is strictly additive for clean input.
#[test]
fn test_owner_target_no_whitespace_baseline() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "feishu:oc_xxx"
            }
        }),
        "reload system.json with clean owner_display succeeds",
    );
    assert_eq!(result, Some(("feishu".to_string(), "oc_xxx".to_string())));
}
