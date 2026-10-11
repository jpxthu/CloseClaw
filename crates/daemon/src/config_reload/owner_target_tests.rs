//! Behavior tests for `parse_owner_target`, co-located with its single
//! definition in `reload.rs` (issue #3241 merged the previously
//! duplicated definitions; the direct test suites moved here so tests
//! and definition live in the same module).
//!
//! Three suites share this module:
//! - the `test_parse_owner_target_*` core cases (six migrated from
//!   `config_reload_tests.rs`)
//! - the two early-exit cases from issue #3250: `get_section_value`
//!   returning `None` (System section never loaded) and the
//!   `SystemConfigData` deserialization miss (malformed `commands`
//!   type) — both silently degrade to `None`
//! - the nine whitespace-dimension cases (issue #3251, migrated from
//!   `config_watcher_owner_target_tests.rs`)
//!
//! Whitespace suite dimensions (behavior, not implementation):
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
//! `owner_display` / no colon / pre-fix empty segments) are covered by
//! the `test_parse_owner_target_*` cases in this same module.

use super::reload::parse_owner_target;
use crate::test_helpers::load_system_config_manager;
use closeclaw_config::manager::{ConfigManager, ConfigSection};
use closeclaw_config::providers::SystemConfigData;
use tempfile::TempDir;

/// Shared setup for all `parse_owner_target` cases in this module: write
/// `system.json` with the per-case payload, load the `System` section
/// into a fresh `ConfigManager`, then parse the owner target from it.
///
/// `reload_expect` is the per-case success message for the mandatory
/// reload step, which this helper checks itself; the per-case `assert_eq!`
/// on the parsed value stays in the calling test.
fn parse_owner_target_from(
    system_json: serde_json::Value,
    reload_expect: &str,
) -> Option<(String, String)> {
    let tmp = TempDir::new().unwrap();
    // Shared primitive: write system.json + new + reload System only;
    // config_dir = tmp root, the layout this helper's callers rely on.
    let cm = load_system_config_manager(tmp.path(), system_json, reload_expect);
    parse_owner_target(&cm)
}

// ---------------------------------------------------------------------------
// parse_owner_target coverage matrix — case × input × branch hit
// (definition: `reload.rs::parse_owner_target`, the single source since
// the issue #3241 merge; branch arms are named semantically so the
// matrix survives line drift.)
//
// test_parse_owner_target_valid                 "feishu:oc_xxx123"  Some (happy path)
// test_parse_owner_target_not_configured        {"version":"1.0"}   `commands?` arm
// test_parse_owner_target_invalid_format        "no-colon-here"     format guard: parts.len() != 2
// test_parse_owner_target_empty_parts           ":oc_xxx"
//     → format guard: platform empty after trim
// test_parse_owner_target_owner_display_missing {"commands":{}}     `owner_display?` arm
// test_parse_owner_target_empty_second_part     "feishu:"
//     → format guard: chat_id empty after trim
// test_parse_owner_target_section_not_loaded    (System never reloaded)
//     → `get_section_value` None arm
// test_parse_owner_target_malformed_commands_type {"commands":"oops"}
//     → `from_value(..).ok()` miss arm
// (The `owner_display_missing` / `empty_second_part` rows were added
// by issue #3243; the last two rows are the issue #3250 early-exit
// arms.)
// ---------------------------------------------------------------------------

/// parse_owner_target correctly parses a valid owner_display value.
#[test]
fn test_parse_owner_target_valid() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "feishu:oc_xxx123"
            }
        }),
        "reload system.json with owner_display succeeds",
    );
    assert_eq!(
        result,
        Some(("feishu".to_string(), "oc_xxx123".to_string()))
    );
}

/// parse_owner_target returns None when owner_display is not configured.
#[test]
fn test_parse_owner_target_not_configured() {
    // Payload omits owner_display
    let result = parse_owner_target_from(
        serde_json::json!({ "version": "1.0" }),
        "reload system.json without owner_display succeeds",
    );
    assert_eq!(result, None);
}

/// parse_owner_target returns None for invalid owner_display format.
#[test]
fn test_parse_owner_target_invalid_format() {
    // Missing colon separator
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "no-colon-here"
            }
        }),
        "reload system.json with malformed owner_display succeeds",
    );
    assert_eq!(result, None);
}

/// parse_owner_target returns None when owner_display has empty parts.
#[test]
fn test_parse_owner_target_empty_parts() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": ":oc_xxx"
            }
        }),
        "reload system.json with empty owner_display parts succeeds",
    );
    assert_eq!(result, None);
}

/// parse_owner_target returns None when `commands` is present but
/// `owner_display` is absent (`{"commands":{}}`): per-field serde
/// defaults yield `commands = Some(...)` with a field-defaulted
/// `owner_display = None`, so the `owner_display?` early return fires —
/// distinct from `test_parse_owner_target_not_configured`, which
/// short-circuits one line earlier on `commands?` and never reaches the
/// `parts` validation.
#[test]
fn test_parse_owner_target_owner_display_missing() {
    // Self-proof before the helper call: the payload must deserialize
    // into SystemConfigData with `commands = Some(..)` and
    // `owner_display = None`, so the None asserted below necessarily
    // comes from the `owner_display?` early return in
    // `reload.rs::parse_owner_target`, not from the `.ok()?`
    // deserialization miss — without this, a later field change could
    // keep the test green while it silently stops covering the target
    // branch.
    let payload = serde_json::json!({ "commands": {} });
    let data: SystemConfigData = serde_json::from_value(payload.clone())
        .expect("payload deserializes into SystemConfigData");
    let commands = data
        .commands
        .expect("`commands` object present → commands = Some(..)");
    assert_eq!(
        commands.owner_display, None,
        "owner_display field-defaults to None, so parse hits owner_display?"
    );

    let result = parse_owner_target_from(
        payload,
        "reload system.json with commands but no owner_display succeeds",
    );
    assert_eq!(result, None);
}

/// parse_owner_target returns None when the target segment after the
/// colon is empty (`"feishu:"` → `splitn(2, ':')` yields `["feishu", ""]`):
/// `parts.len() == 2` holds but `parts[1].is_empty()` trips the format
/// guard — the symmetric counterpart of `test_parse_owner_target_empty_parts`
/// (leading-empty `":oc_xxx"`), covering the trailing-empty edge.
#[test]
fn test_parse_owner_target_empty_second_part() {
    let result = parse_owner_target_from(
        serde_json::json!({
            "commands": {
                "ownerDisplay": "feishu:"
            }
        }),
        "reload system.json with empty target part succeeds",
    );
    assert_eq!(result, None);
}

/// parse_owner_target returns None when the System section was never
/// loaded into the manager (`get_section_value` miss — the not-loaded
/// leg of issue #3250): a fresh manager over a config dir with no
/// reload at all has an empty section cache, so the chain short-circuits
/// on `get_section_value(..)?` before any deserialization or field
/// access.
#[test]
fn test_parse_owner_target_section_not_loaded() {
    let tmp = TempDir::new().unwrap();
    let cm = ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new");

    // Self-proof before the call: the System section is absent from the
    // cache and the section is *not* blocked, so the None asserted below
    // necessarily comes from the `get_section_value` not-loaded miss,
    // not from a blocked section — without this, a changed block
    // default could keep the test green while it silently stops
    // covering the target leg.
    assert!(
        cm.get_section_value(ConfigSection::System).is_none(),
        "System section must be absent from the cache before any reload"
    );
    assert!(
        !cm.is_blocked(ConfigSection::System),
        "System section must not be blocked in the not-loaded case"
    );

    assert_eq!(parse_owner_target(&cm), None);
}

/// parse_owner_target returns None when the System section holds a
/// `commands` field of the wrong type (`{"commands": "oops"}` — the
/// deserialization-miss leg of issue #3250): `reload_section` only
/// JSON-parses the file, so the malformed section value lands in the
/// cache and the chain must bail on the
/// `serde_json::from_value(..).ok()?` arm — malformed config silently
/// degrades to "not configured" instead of failing the reload path.
#[test]
fn test_parse_owner_target_malformed_commands_type() {
    // Self-proof before the helper call: `commands` as a plain string
    // fails SystemConfigData deserialization (the field is a typed
    // struct), so the None asserted below necessarily comes from the
    // `.ok()?` deserialization miss, not from the `commands?` or
    // `owner_display?` early returns — without this, a serde change
    // (e.g. an untagged string fallback) could keep the test green
    // while it silently stops covering the target branch.
    let payload = serde_json::json!({ "commands": "oops" });
    assert!(
        serde_json::from_value::<SystemConfigData>(payload.clone()).is_err(),
        "`commands` as a string must fail SystemConfigData deserialization"
    );

    // The reload itself must succeed: the file is valid JSON and the
    // daemon reload path passes no validator, so the malformed value
    // really reaches parse_owner_target through the section cache.
    let result = parse_owner_target_from(
        payload,
        "reload system.json with malformed commands type succeeds",
    );
    assert_eq!(result, None);
}

// ---------------------------------------------------------------------------
// parse_owner_target whitespace coverage matrix — case × input × branch hit
// (same single definition `reload.rs::parse_owner_target`; branch arms
// named semantically as in the core matrix above)
//
//   test_owner_target_whitespace_after_colon_normalized    "feishu: oc"
//     → Some (segments trimmed)
//   test_owner_target_whitespace_before_colon_normalized   " feishu:oc"
//     → Some
//   test_owner_target_whitespace_platform_side_normalized  "feishu :oc"
//     → Some
//   test_owner_target_ideographic_space_normalized         "　feishu:oc　"
//     → Some (trim covers U+3000)
//   test_owner_target_whitespace_chat_id_rejected          "feishu: "
//     → format guard: chat_id empty after trim → None
//   test_owner_target_whitespace_platform_rejected         " :oc"
//     → format guard: platform empty after trim → None
//   test_owner_target_all_whitespace_pair_rejected         " : "
//     → format guard: platform empty after trim → None
//   test_owner_target_pure_whitespace_rejected             "   "
//     → format guard: parts.len() != 2 → None
//   test_owner_target_no_whitespace_baseline               "feishu:oc_xxx"
//     → Some (regression baseline)
// ---------------------------------------------------------------------------

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
/// `"no-colon-here"` (covered by `test_parse_owner_target_invalid_format`
/// in this module).
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
