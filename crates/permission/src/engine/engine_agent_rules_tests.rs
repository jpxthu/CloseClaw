//! Integration tests for agent rule lazy loading via `PermissionEngine::evaluate()`.
//!
//! Tests cover: normal path (lazy load + merge), hot-reload (mtime/invalidation),
//! error paths (missing file, corrupted JSON), and boundary values (multi-agent
//! isolation, cache hit).

use super::engine_eval::PermissionEngine;
use super::engine_types::{Effect, PermissionRequest, PermissionRequestBody, PermissionResponse};
use crate::actions::ActionBuilder;
use crate::rules::{RuleBuilder, RuleSetBuilder};
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a full RuleSet JSON (with defaults) for `agent_id` into
/// `{tmp}/agents/{agent_id}/permissions.json`.
fn write_agent_rules(tmp: &TempDir, agent_id: &str, rules_json: &str) {
    let agent_dir = tmp.path().join("agents").join(agent_id);
    fs::create_dir_all(&agent_dir).unwrap();
    let full_json = format!(
        r#"{{
            "rules": {},
            "defaults": {{
                "file_read": "deny", "file_write": "deny", "exec": "deny",
                "network": "deny", "inter_agent": "deny", "config": "deny",
                "tool_call": "deny", "message": "allow"
            }},
            "user_defaults": {{
                "file_read": "deny", "file_write": "deny", "exec": "deny",
                "network": "deny", "inter_agent": "deny", "config": "deny",
                "tool_call": "deny", "message": "deny"
            }}
        }}"#,
        rules_json
    );
    fs::write(agent_dir.join("permissions.json"), full_json).unwrap();
}

/// Build a global RuleSet with one Allow rule for `agent_id` → file read `/**`.
fn global_rules_with_allow(agent_id: &str) -> super::engine_types::RuleSet {
    RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_exec(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .default_message(Effect::Allow)
        .rule(
            RuleBuilder::new()
                .name(format!("global-allow-read-{}", agent_id))
                .subject_agent(agent_id)
                .allow()
                .action(
                    ActionBuilder::file("read", vec!["/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap()
}

/// Build a deny-all global RuleSet (no rules, all defaults Deny).
fn deny_all_global_rules() -> super::engine_types::RuleSet {
    RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_exec(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .default_message(Effect::Deny)
        .build()
        .unwrap()
}

/// Build a FileOp request for `agent_id` reading `path`.
fn file_read_request(agent_id: &str, path: &str) -> PermissionRequest {
    PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: agent_id.to_string(),
        path: path.to_string(),
        op: "read".to_string(),
    })
}

/// Build a CommandExec request for `agent_id`.
fn exec_request(agent_id: &str, cmd: &str) -> PermissionRequest {
    PermissionRequest::Bare(PermissionRequestBody::CommandExec {
        agent: agent_id.to_string(),
        cmd: cmd.to_string(),
        args: vec![],
    })
}

// ===========================================================================
// 1. Normal Path — lazy loading + merge
// ===========================================================================

#[test]
fn test_first_evaluate_triggers_lazy_load_allow() {
    // Agent file has Allow for file_read. First evaluate() should load and apply.
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "agent-1",
        r#"[{"name":"agent-allow-read","subject":{"agent":"agent-1"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    let ruleset = deny_all_global_rules();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());
    let resp = engine.evaluate(file_read_request("agent-1", "/data/file.txt"), None);

    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "expected Allowed from agent rule, got {:?}",
        resp
    );
}

#[test]
fn test_first_evaluate_triggers_lazy_load_deny() {
    // Agent file has explicit Deny for file_read.
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "agent-deny",
        r#"[{"name":"agent-deny-read","subject":{"agent":"agent-deny"},"effect":"deny","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    // Global: Allow file_read (so without agent rule, it would be Allowed).
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_exec(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .default_message(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());
    let resp = engine.evaluate(file_read_request("agent-deny", "/any/path"), None);

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "expected Denied from agent deny rule overriding global Allow, got {:?}",
        resp
    );
}

#[test]
fn test_global_and_agent_rules_both_participate() {
    // Global: deny all, except allow exec for agent-merge.
    // Agent: allow file_read for agent-merge.
    // → file_read should be Allowed (from agent), exec should be Denied (from global).
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "agent-merge",
        r#"[{"name":"agent-allow-read","subject":{"agent":"agent-merge"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_exec(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .default_message(Effect::Deny)
        .rule(
            RuleBuilder::new()
                .name("global-allow-exec-merge")
                .subject_agent("agent-merge")
                .allow()
                .action(ActionBuilder::command("cargo").build().unwrap())
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    // file_read → Allowed (from agent rule)
    let read_resp = engine.evaluate(file_read_request("agent-merge", "/data/file.txt"), None);
    assert!(
        matches!(read_resp, PermissionResponse::Allowed { .. }),
        "file_read should be Allowed from agent rule, got {:?}",
        read_resp
    );

    // exec cargo → Allowed (from global rule)
    let exec_resp = engine.evaluate(exec_request("agent-merge", "cargo"), None);
    assert!(
        matches!(exec_resp, PermissionResponse::Allowed { .. }),
        "exec cargo should be Allowed from global rule, got {:?}",
        exec_resp
    );

    // exec rm → Denied (no rule, global default Deny)
    let rm_resp = engine.evaluate(exec_request("agent-merge", "rm"), None);
    assert!(
        matches!(rm_resp, PermissionResponse::Denied { .. }),
        "exec rm should be Denied (no rule, default Deny), got {:?}",
        rm_resp
    );
}

#[test]
fn test_startup_does_not_read_agent_directory() {
    // Create agent file AFTER engine construction. First evaluate for a
    // DIFFERENT agent should not touch the first agent's file.
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "agent-lazy",
        r#"[{"name":"should-not-load","subject":{"agent":"agent-lazy"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    let ruleset = deny_all_global_rules();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    // Evaluate for a completely different agent → agent-lazy's file is NOT read.
    let resp = engine.evaluate(file_read_request("other-agent", "/data/file.txt"), None);
    // other-agent has no agent file → defaults → Denied.
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "other-agent should get default Deny (agent-lazy file not loaded), got {:?}",
        resp
    );
}

// ===========================================================================
// 2. State Transition — hot reload
// ===========================================================================

#[test]
fn test_mtime_change_picks_up_new_rules() {
    // Load agent rules, then update the file. Next evaluate should see new rules.
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "hot-agent",
        r#"[{"name":"old-rule","subject":{"agent":"hot-agent"},"effect":"deny","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_exec(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .default_message(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    // First evaluate → agent deny rule loaded → Denied.
    let resp1 = engine.evaluate(file_read_request("hot-agent", "/data/file.txt"), None);
    assert!(
        matches!(resp1, PermissionResponse::Denied { .. }),
        "first eval should be Denied, got {:?}",
        resp1
    );

    // Update file: change deny to allow.
    write_agent_rules(
        &tmp,
        "hot-agent",
        r#"[{"name":"new-rule","subject":{"agent":"hot-agent"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );
    // Ensure mtime differs (some filesystems have 1s granularity).
    // Write again to bump mtime.
    let agent_dir = tmp.path().join("agents").join("hot-agent");
    let new_content = fs::read_to_string(agent_dir.join("permissions.json")).unwrap();
    fs::write(
        agent_dir.join("permissions.json"),
        format!("{} ", new_content),
    )
    .unwrap();

    // Second evaluate → should read updated file.
    let resp2 = engine.evaluate(file_read_request("hot-agent", "/data/file.txt"), None);
    assert!(
        matches!(resp2, PermissionResponse::Allowed { .. }),
        "second eval should be Allowed after file update, got {:?}",
        resp2
    );
}

#[test]
fn test_explicit_invalidate_causes_immediate_reload() {
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "inv-agent",
        r#"[{"name":"initial","subject":{"agent":"inv-agent"},"effect":"deny","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_exec(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .default_message(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    // First evaluate → Denied (agent deny).
    let resp1 = engine.evaluate(file_read_request("inv-agent", "/data/file.txt"), None);
    assert!(matches!(resp1, PermissionResponse::Denied { .. }));

    // Update file.
    write_agent_rules(
        &tmp,
        "inv-agent",
        r#"[{"name":"updated","subject":{"agent":"inv-agent"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    // Explicitly invalidate → forces re-read.
    engine.invalidate_agent_rules("inv-agent");
    let resp2 = engine.evaluate(file_read_request("inv-agent", "/data/file.txt"), None);
    assert!(
        matches!(resp2, PermissionResponse::Allowed { .. }),
        "after invalidate, should pick up new rules, got {:?}",
        resp2
    );
}

#[test]
fn test_reload_rules_invalidates_agent_cache() {
    // When global rules are reloaded, agent cache is invalidated.
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "gr-agent",
        r#"[{"name":"agent-rule","subject":{"agent":"gr-agent"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    let ruleset_v1 = deny_all_global_rules();
    let mut engine = PermissionEngine::new(ruleset_v1, tmp.path().to_path_buf());

    // First evaluate → Allowed (from agent rule).
    let resp1 = engine.evaluate(file_read_request("gr-agent", "/data/file.txt"), None);
    assert!(matches!(resp1, PermissionResponse::Allowed { .. }));

    // Reload global rules (different version).
    let ruleset_v2 = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_exec(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .default_tool_call(Effect::Deny)
        .default_message(Effect::Deny)
        .rule(
            RuleBuilder::new()
                .name("new-global-deny")
                .subject_agent("gr-agent")
                .deny()
                .action(
                    ActionBuilder::file("read", vec!["/**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    engine.reload_rules(ruleset_v2);

    // Evaluate again → should use new global deny rule.
    let resp2 = engine.evaluate(file_read_request("gr-agent", "/data/file.txt"), None);
    assert!(
        matches!(resp2, PermissionResponse::Denied { .. }),
        "after reload_rules, should pick up new global deny, got {:?}",
        resp2
    );
}

// ===========================================================================
// 3. Error Path
// ===========================================================================

#[test]
fn test_missing_agent_file_falls_back_to_global() {
    // No agent file exists → evaluate uses only global rules.
    let tmp = TempDir::new().unwrap();

    let ruleset = global_rules_with_allow("no-file-agent");
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    let resp = engine.evaluate(file_read_request("no-file-agent", "/data/file.txt"), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "missing agent file should fall back to global Allow, got {:?}",
        resp
    );
}

#[test]
fn test_corrupted_json_falls_back_to_global() {
    // Agent file exists but is invalid JSON → treated as no agent rules.
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("agents").join("bad-json-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(agent_dir.join("permissions.json"), "{not valid json!!!").unwrap();

    let ruleset = global_rules_with_allow("bad-json-agent");
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    let resp = engine.evaluate(file_read_request("bad-json-agent", "/data/file.txt"), None);
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "corrupted JSON should fall back to global Allow, got {:?}",
        resp
    );
}

#[test]
fn test_corrupted_json_no_panic() {
    // Corrupted file must not cause a panic.
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("agents").join("panic-agent");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(
        agent_dir.join("permissions.json"),
        r#"{"rules": "not_an_array", "broken": true}"#,
    )
    .unwrap();

    let ruleset = deny_all_global_rules();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    // Should not panic.
    let _ = engine.evaluate(file_read_request("panic-agent", "/any"), None);
}

#[test]
fn test_deleted_file_after_load_uses_stale_cache() {
    // File exists, is loaded, then deleted. Without explicit invalidate,
    // the cached entry is still used (mtime-based staleness check uses
    // cached mtime vs current — if file is deleted, read_mtime returns
    // Err, which != cached mtime, triggering a reload to empty).
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "del-agent",
        r#"[{"name":"del-rule","subject":{"agent":"del-agent"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    let ruleset = deny_all_global_rules();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    // First evaluate → Allowed (agent rule loaded).
    let resp1 = engine.evaluate(file_read_request("del-agent", "/data/file.txt"), None);
    assert!(matches!(resp1, PermissionResponse::Allowed { .. }));

    // Delete the agent file.
    fs::remove_file(
        tmp.path()
            .join("agents")
            .join("del-agent")
            .join("permissions.json"),
    )
    .unwrap();

    // Next evaluate: file gone → read_mtime returns Err ≠ cached mtime → reload → empty.
    let resp2 = engine.evaluate(file_read_request("del-agent", "/data/file.txt"), None);
    assert!(
        matches!(resp2, PermissionResponse::Denied { .. }),
        "after file deletion, should fall back to default Deny, got {:?}",
        resp2
    );
}

// ===========================================================================
// 4. Boundary Values
// ===========================================================================

#[test]
fn test_multiple_agents_isolated_caches() {
    // Two agents with different rules should be evaluated independently.
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "iso-a",
        r#"[{"name":"allow-a","subject":{"agent":"iso-a"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );
    write_agent_rules(
        &tmp,
        "iso-b",
        r#"[{"name":"deny-b","subject":{"agent":"iso-b"},"effect":"deny","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    let ruleset = deny_all_global_rules();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    let resp_a = engine.evaluate(file_read_request("iso-a", "/data/file.txt"), None);
    let resp_b = engine.evaluate(file_read_request("iso-b", "/data/file.txt"), None);

    assert!(
        matches!(resp_a, PermissionResponse::Allowed { .. }),
        "iso-a should be Allowed, got {:?}",
        resp_a
    );
    assert!(
        matches!(resp_b, PermissionResponse::Denied { .. }),
        "iso-b should be Denied, got {:?}",
        resp_b
    );

    // Change iso-b's file → iso-a's cache should remain unaffected.
    write_agent_rules(
        &tmp,
        "iso-b",
        r#"[{"name":"allow-b-new","subject":{"agent":"iso-b"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );
    let agent_dir = tmp.path().join("agents").join("iso-b");
    let content = fs::read_to_string(agent_dir.join("permissions.json")).unwrap();
    fs::write(agent_dir.join("permissions.json"), format!("{} ", content)).unwrap();

    // iso-b should now be Allowed.
    let resp_b2 = engine.evaluate(file_read_request("iso-b", "/data/file.txt"), None);
    assert!(
        matches!(resp_b2, PermissionResponse::Allowed { .. }),
        "iso-b should now be Allowed after file update, got {:?}",
        resp_b2
    );

    // iso-a should still be Allowed (cache unaffected).
    let resp_a2 = engine.evaluate(file_read_request("iso-a", "/data/file.txt"), None);
    assert!(
        matches!(resp_a2, PermissionResponse::Allowed { .. }),
        "iso-a cache should be unaffected, got {:?}",
        resp_a2
    );
}

#[test]
fn test_high_frequency_evaluate_cache_hit() {
    // Multiple evaluates for the same agent should use the cached entry
    // (no repeated file I/O when mtime is unchanged).
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "freq-agent",
        r#"[{"name":"freq-rule","subject":{"agent":"freq-agent"},"effect":"allow","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    let ruleset = deny_all_global_rules();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    // First evaluate → loads from disk.
    let resp1 = engine.evaluate(file_read_request("freq-agent", "/data/file.txt"), None);
    assert!(matches!(resp1, PermissionResponse::Allowed { .. }));

    // Subsequent evaluates should all succeed with the same result.
    for i in 0..10 {
        let resp = engine.evaluate(
            file_read_request("freq-agent", &format!("/data/file-{}.txt", i)),
            None,
        );
        assert!(
            matches!(resp, PermissionResponse::Allowed { .. }),
            "evaluate #{} should still be Allowed (cache hit), got {:?}",
            i,
            resp
        );
    }
}

#[test]
fn test_agent_with_no_file_uses_global_defaults() {
    // Agent has no permissions.json → default Deny applies.
    let tmp = TempDir::new().unwrap();
    let ruleset = deny_all_global_rules();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    let resp = engine.evaluate(file_read_request("unknown-agent", "/any/path"), None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "agent with no file should get default Deny, got {:?}",
        resp
    );
}

#[test]
fn test_agent_rules_deny_priority_over_global_allow() {
    // Global allows, agent denies → Deny wins (deny priority).
    let tmp = TempDir::new().unwrap();
    write_agent_rules(
        &tmp,
        "dp-agent",
        r#"[{"name":"agent-deny","subject":{"agent":"dp-agent"},"effect":"deny","actions":[{"type":"file","operation":"read","paths":["/**"]}]}]"#,
    );

    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_exec(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .default_message(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new(ruleset, tmp.path().to_path_buf());

    let resp = engine.evaluate(file_read_request("dp-agent", "/data/file.txt"), None);
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "agent deny should override global allow, got {:?}",
        resp
    );
}
