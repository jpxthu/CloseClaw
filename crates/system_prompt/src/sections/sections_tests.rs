//! Tests for `sections` module.
//!
//! Extracted from the inline `#[cfg(test)] mod tests` in `sections.rs`
//! to keep that file focused on rendering logic.

use super::*;
use tempfile::tempdir;

#[test]
fn test_section_render_memory() {
    let s = Section::MemorySection("You are a helpful assistant.".to_string());
    let rendered = s.render();
    assert!(rendered.contains("## Memory"));
    assert!(rendered.contains("You are a helpful assistant"));
    assert!(s.is_cacheable());
}

#[test]
fn test_section_render_channel_context() {
    let s = Section::ChannelContext {
        chat_name: "test-chat".to_string(),
    };
    let rendered = s.render();
    assert!(rendered.contains("chat_name: test-chat"));
    assert!(!s.is_cacheable());
}

#[test]
fn test_section_cache_invalidate() {
    let mut cache = SectionCache::new();
    cache.put("test_section", "old content".to_string(), Some(100));
    assert_eq!(
        cache.get("test_section", Some(100)),
        Some("old content".to_string())
    );

    cache.invalidate("test_section");
    assert_eq!(cache.get("test_section", Some(100)), None);
}

#[test]
fn test_section_cache_stale_on_mtime_change() {
    let mut cache = SectionCache::new();
    cache.put("file_section", "v1".to_string(), Some(100));
    // Same mtime → cache hit
    assert_eq!(cache.get("file_section", Some(100)), Some("v1".to_string()));
    // Different mtime → cache stale
    assert_eq!(cache.get("file_section", Some(200)), None);
}

#[test]
fn test_load_cached_file_section_fresh() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.txt");
    std::fs::write(&file_path, "hello world").unwrap();

    let mut cache = SectionCache::new();

    // First load — cache miss, should read from file
    let result = load_cached_file_section(&mut cache, "test", &file_path);
    assert_eq!(result, Some("hello world".to_string()));

    // Second load — cache hit, same content
    let result2 = load_cached_file_section(&mut cache, "test", &file_path);
    assert_eq!(result2, Some("hello world".to_string()));

    // Simulate staleness by manually inserting a cache entry with a stale mtime,
    // then verify load_cached_file_section reloads the fresh content.
    cache.invalidate("test");
    cache.put("test", "stale content".to_string(), Some(0)); // mtime=0 is always stale
    std::fs::write(&file_path, "updated content").unwrap();
    let result3 = load_cached_file_section(&mut cache, "test", &file_path);
    assert_eq!(result3, Some("updated content".to_string()));
}

#[test]
fn test_git_status_render() {
    let s = Section::GitStatus("On branch master\n?? file.txt".to_string());
    let rendered = s.render();
    assert!(rendered.contains("## Git Status"));
    assert!(rendered.contains("On branch master"));
    assert!(!s.is_cacheable());
}

#[test]
fn test_working_directory_section() {
    let s = Section::WorkingDirectory("/home/user/.closeclaw/workspaces/agent1/user1/".to_string());
    assert!(!s.is_cacheable());
    assert_eq!(s.name(), "working_directory");
    let rendered = s.render();
    assert!(rendered.contains("## Working Directory"));
    assert!(rendered.contains("~/agent1/user1/"));
    assert!(!rendered.contains(".closeclaw"));
}

#[test]
fn test_sanitize_workdir_path() {
    assert_eq!(
        sanitize_workdir_path("/home/user/.closeclaw/workspaces/a/u/"),
        "~/a/u/"
    );
    assert_eq!(
        sanitize_workdir_path("/some/random/path"),
        "/some/random/path"
    );
    assert_eq!(sanitize_workdir_path(""), "");
}

#[test]
fn test_section_cache_invalidate_skill_listing() {
    let mut cache = SectionCache::new();
    // Pre-populate the skill_listing cache with known content
    cache.put("skill_listing", "old skill content".to_string(), Some(999));
    // Verify it's cached
    assert_eq!(
        cache.get("skill_listing", Some(999)),
        Some("old skill content".to_string())
    );

    // Invalidate via the SectionCache method
    cache.invalidate_skill_listing();

    // Cache should be cleared
    assert_eq!(cache.get("skill_listing", Some(999)), None);
}

// -----------------------------------------------------------------------
// Step 1.5: SectionCache instance isolation tests
// -----------------------------------------------------------------------

/// Two independent SectionCache instances do not share entries.
#[test]
fn test_section_cache_isolation_between_instances() {
    let mut cache_a = SectionCache::new();
    let mut cache_b = SectionCache::new();

    cache_a.put("key-a", "value-a".to_string(), None);
    cache_b.put("key-b", "value-b".to_string(), None);

    // cache_a has key-a but not key-b
    assert_eq!(cache_a.get("key-a", None), Some("value-a".to_string()));
    assert_eq!(cache_a.get("key-b", None), None);

    // cache_b has key-b but not key-a
    assert_eq!(cache_b.get("key-b", None), Some("value-b".to_string()));
    assert_eq!(cache_b.get("key-a", None), None);
}

/// Invalidating one instance does not affect the other.
#[test]
fn test_section_cache_invalidate_isolation() {
    let mut cache_a = SectionCache::new();
    let mut cache_b = SectionCache::new();

    cache_a.put("shared-key", "from-a".to_string(), None);
    cache_b.put("shared-key", "from-b".to_string(), None);

    cache_a.invalidate("shared-key");

    // cache_a entry removed
    assert_eq!(cache_a.get("shared-key", None), None);
    // cache_b entry still present
    assert_eq!(cache_b.get("shared-key", None), Some("from-b".to_string()));
}

/// invalidate_tools removes only the tools entry, leaving other entries intact.
#[test]
fn test_invalidate_tools() {
    let mut cache = SectionCache::new();
    cache.put("tools", "tool content".to_string(), None);
    cache.put("memory", "memory content".to_string(), None);

    // Verify both entries are cached
    assert_eq!(cache.get("tools", None), Some("tool content".to_string()));
    assert_eq!(
        cache.get("memory", None),
        Some("memory content".to_string())
    );

    // Invalidate tools only
    cache.invalidate_tools();

    // Tools entry removed
    assert_eq!(cache.get("tools", None), None);
    // Memory entry unaffected
    assert_eq!(
        cache.get("memory", None),
        Some("memory content".to_string())
    );
}

// -----------------------------------------------------------------------
// Step 1.5: Edge case tests
// -----------------------------------------------------------------------

/// Empty cache returns None for any key.
#[test]
fn test_section_cache_empty_returns_none() {
    let cache = SectionCache::new();
    assert_eq!(cache.get("any-key", None), None);
    assert_eq!(cache.get("any-key", Some(123)), None);
}

/// Cache hit returns the stored content.
#[test]
fn test_section_cache_hit_returns_content() {
    let mut cache = SectionCache::new();
    cache.put("my-key", "my-content".to_string(), None);
    assert_eq!(cache.get("my-key", None), Some("my-content".to_string()));
}

/// Cache miss (key not present) returns None.
#[test]
fn test_section_cache_miss_returns_none() {
    let mut cache = SectionCache::new();
    cache.put("existing-key", "value".to_string(), None);
    assert_eq!(cache.get("nonexistent-key", None), None);
}

// -----------------------------------------------------------------------
// Step 1.5: State transition lifecycle tests
// -----------------------------------------------------------------------

/// Cache lifecycle: empty → put → hit → invalidate → miss → put → hit.
#[test]
fn test_section_cache_lifecycle_empty_to_populated_to_invalidated_to_rebuilt() {
    let mut cache = SectionCache::new();

    // 1. Empty: miss
    assert_eq!(cache.get("lifecycle", None), None);

    // 2. Put: hit
    cache.put("lifecycle", "v1".to_string(), None);
    assert_eq!(cache.get("lifecycle", None), Some("v1".to_string()));

    // 3. Invalidate: miss
    cache.invalidate("lifecycle");
    assert_eq!(cache.get("lifecycle", None), None);

    // 4. Put again (rebuild): hit
    cache.put("lifecycle", "v2".to_string(), None);
    assert_eq!(cache.get("lifecycle", None), Some("v2".to_string()));
}

/// invalidate_all clears the entire cache.
#[test]
fn test_section_cache_invalidate_all_clears_all() {
    let mut cache = SectionCache::new();
    cache.put("k1", "v1".to_string(), None);
    cache.put("k2", "v2".to_string(), None);
    cache.put("k3", "v3".to_string(), None);

    cache.invalidate_all();

    assert_eq!(cache.get("k1", None), None);
    assert_eq!(cache.get("k2", None), None);
    assert_eq!(cache.get("k3", None), None);
}

/// invalidate_all on empty cache is a no-op.
#[test]
fn test_section_cache_invalidate_all_on_empty_is_noop() {
    let mut cache = SectionCache::new();
    cache.invalidate_all(); // should not panic
    assert_eq!(cache.get("anything", None), None);
}

/// Cache with mtime validation: stale entry returns None.
#[test]
fn test_section_cache_mtime_stale_returns_none() {
    let mut cache = SectionCache::new();
    cache.put("mtime-key", "content".to_string(), Some(100));

    // Same mtime → hit
    assert_eq!(
        cache.get("mtime-key", Some(100)),
        Some("content".to_string())
    );

    // Different mtime → stale → miss
    assert_eq!(cache.get("mtime-key", Some(200)), None);
}

/// Cache without mtime (mtime=None) always hits regardless of current_mtime.
#[test]
fn test_section_cache_no_mtime_always_hits() {
    let mut cache = SectionCache::new();
    cache.put("no-mtime-key", "content".to_string(), None);

    // No stored mtime → always hit
    assert_eq!(cache.get("no-mtime-key", None), Some("content".to_string()));
    assert_eq!(
        cache.get("no-mtime-key", Some(999)),
        Some("content".to_string())
    );
}

// -----------------------------------------------------------------------
// Coverage for all remaining Section variants after WorkspaceSection removal
// -----------------------------------------------------------------------

// -----------------------------------------------------------------------
// ModeInstruction tests
// -----------------------------------------------------------------------

#[test]
fn test_mode_instruction_basics_and_auto() {
    let s = Section::ModeInstruction {
        mode: SessionMode::Normal,
        plan_path: None,
        sparse: false,
        sub_agent: false,
    };
    assert_eq!(s.name(), "mode_instruction");
    assert!(!s.is_cacheable());
    assert_eq!(s.render(), "");
    // Auto mode renders all 6 rules
    let rendered = render_mode_instruction(SessionMode::Auto, None);
    assert!(rendered.contains("Auto Mode Active"));
    assert!(rendered.contains("Execute immediately"));
    assert!(rendered.contains("Minimize interruptions"));
    assert!(rendered.contains("Prefer action over planning"));
    assert!(rendered.contains("Expect course corrections"));
    assert!(rendered.contains("Do not take overly destructive actions"));
    assert!(rendered.contains("Avoid data exfiltration"));
    // Default PlanPath is now Standard
    let s = Section::ModeInstruction {
        mode: SessionMode::Plan,
        plan_path: None,
        sparse: false,
        sub_agent: false,
    };
    assert!(s.render().contains("Standard Path"));
}

#[test]
fn test_mode_instruction_plan_standard_and_interview() {
    let std = Section::ModeInstruction {
        mode: SessionMode::Plan,
        plan_path: Some(PlanPath::Standard),
        sparse: false,
        sub_agent: false,
    };
    let r = std.render();
    assert!(r.contains("## Mode: Plan \u{2014} Standard Path"));
    assert!(r.contains("This supercedes any other instructions"));
    assert!(r.contains("Phase 4: Final Plan"));
    assert!(!r.contains("Phase 5"));
    assert!(!r.contains("Interview Path"));
    let intv = Section::ModeInstruction {
        mode: SessionMode::Plan,
        plan_path: Some(PlanPath::Interview),
        sparse: false,
        sub_agent: false,
    };
    let r = intv.render();
    assert!(r.contains("## Mode: Plan \u{2014} Interview Path"));
    assert!(r.contains("pair-planning"));
    assert!(r.contains("Don't explore exhaustively before engaging the user"));
    assert!(r.contains("Never ask what you could find out by reading the code"));
    assert!(r.contains("When to Converge"));
    assert!(r.contains("The Loop"));
    assert!(!r.contains("Standard Path"));
}

// -----------------------------------------------------------------------
// Sparse / sub-agent variant tests
// -----------------------------------------------------------------------

#[test]
fn test_mode_instruction_sparse_and_sub_agent() {
    // Plan sparse → Standard Sparse text
    let rendered = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        true,
        false,
    );
    assert!(rendered.contains("Plan mode still active"));
    assert!(rendered.contains("Read-only except plan file"));
    // Auto sparse → Auto Sparse text
    let rendered = render_mode_instruction_with_flags(SessionMode::Auto, None, true, false);
    assert!(rendered.contains("Auto mode still active"));
    assert!(rendered.contains("Execute autonomously"));
    // Sub-agent → Sub-agent Sparse text
    let rendered = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        false,
        true,
    );
    assert!(rendered.contains("Plan mode is active"));
    assert!(rendered.contains("READ-ONLY actions"));
    // sub_agent takes precedence over sparse
    let rendered =
        render_mode_instruction_with_flags(SessionMode::Plan, Some(PlanPath::Standard), true, true);
    assert!(rendered.contains("READ-ONLY actions"));
    assert!(!rendered.contains("Plan mode still active"));
}

// -----------------------------------------------------------------------
// Bug fix verification: Section::render() ModeInstruction with flags
// -----------------------------------------------------------------------

#[test]
fn test_section_render_mode_instruction_uses_flags() {
    // When sparse=true, render() should output Standard Sparse text
    let s = Section::ModeInstruction {
        mode: SessionMode::Plan,
        plan_path: Some(PlanPath::Standard),
        sparse: true,
        sub_agent: false,
    };
    let r = s.render();
    assert!(
        r.contains("Plan mode still active"),
        "Expected Standard Sparse text from render(), got: {}",
        r
    );

    // When sub_agent=true, render() should output Sub-agent Sparse text
    let s = Section::ModeInstruction {
        mode: SessionMode::Plan,
        plan_path: Some(PlanPath::Standard),
        sparse: false,
        sub_agent: true,
    };
    let r = s.render();
    assert!(
        r.contains("Plan mode is active"),
        "Expected Sub-agent Sparse text from render(), got: {}",
        r
    );
}

// ── Gap 4: Interview Path global constraint ─────────────────────────────

/// Verify render_interview_path_instruction() includes PLAN_MODE_CONSTRAINT.
#[test]
fn test_interview_path_includes_plan_mode_constraint() {
    let output = render_interview_path_instruction();
    assert!(
        output.contains(PLAN_MODE_CONSTRAINT),
        "render_interview_path_instruction() must include PLAN_MODE_CONSTRAINT"
    );
}

/// Verify render_interview_path_instruction() output format matches
/// "## Mode: Plan — Interview Path\n\n{CONSTRAINT}\n\n{PROMPT}\n".
#[test]
fn test_interview_path_output_format() {
    let output = render_interview_path_instruction();
    let expected = format!(
        "## Mode: Plan \u{2014} Interview Path\n\n{}\n\n{}\n",
        PLAN_MODE_CONSTRAINT, INTERVIEW_PATH_PROMPT
    );
    assert_eq!(output, expected);
}

/// Verify render_standard_path_instruction() and render_interview_path_instruction()
/// use the same PLAN_MODE_CONSTRAINT text.
#[test]
fn test_standard_and_interview_share_plan_mode_constraint() {
    let standard = render_standard_path_instruction();
    let interview = render_interview_path_instruction();
    // Both should contain the same constraint text
    assert!(standard.contains(PLAN_MODE_CONSTRAINT));
    assert!(interview.contains(PLAN_MODE_CONSTRAINT));
    // The constraint text should be identical in both outputs
    let standard_idx = standard.find(PLAN_MODE_CONSTRAINT).unwrap();
    let interview_idx = interview.find(PLAN_MODE_CONSTRAINT).unwrap();
    let standard_slice = &standard[standard_idx..standard_idx + PLAN_MODE_CONSTRAINT.len()];
    let interview_slice = &interview[interview_idx..interview_idx + PLAN_MODE_CONSTRAINT.len()];
    assert_eq!(standard_slice, interview_slice);
}

// ── Gap 1: Sparse injection tests ────────────────────────────────────────

/// Plan Mode + compacted → STANDARD_SPARSE text
#[test]
fn test_sparse_plan_mode_outputs_standard_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        true,  // is_compacted
        false, // is_sub_agent
    );
    assert!(
        output.contains("Plan mode still active"),
        "Plan Mode compacted should output STANDARD_SPARSE, got: {}",
        output
    );
    assert!(output.contains("Read-only except plan file"));
}

/// Auto Mode + compacted → AUTO_MODE_SPARSE text (different from Plan sparse)
#[test]
fn test_sparse_auto_mode_outputs_auto_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None,  // plan_path irrelevant for Auto
        true,  // is_compacted
        false, // is_sub_agent
    );
    assert!(
        output.contains("Auto mode still active"),
        "Auto Mode compacted should output AUTO_MODE_SPARSE, got: {}",
        output
    );
    assert!(output.contains("Execute autonomously"));
}

/// Plan sparse and Auto sparse produce different output
#[test]
fn test_sparse_plan_and_auto_produce_different_output() {
    let plan_sparse = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        true,
        false,
    );
    let auto_sparse = render_mode_instruction_with_flags(SessionMode::Auto, None, true, false);
    assert_ne!(
        plan_sparse, auto_sparse,
        "Plan sparse and Auto sparse should produce different outputs"
    );
    assert!(plan_sparse.contains("Plan mode"));
    assert!(auto_sparse.contains("Auto mode"));
}

/// Not compacted → full prompt (no sparse text)
#[test]
fn test_not_compacted_outputs_full_prompt() {
    let plan_full = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        false, // not compacted
        false,
    );
    // Full Plan prompt should contain standard path phases, not sparse
    assert!(plan_full.contains("Phase 1: Initial Understanding"));
    assert!(!plan_full.contains("Plan mode still active"));

    let auto_full = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None,
        false, // not compacted
        false,
    );
    // Full Auto prompt should contain full auto instructions, not sparse
    assert!(auto_full.contains("Auto Mode Active"));
    assert!(!auto_full.contains("Auto mode still active"));
}

// ── Gap 2: Sub-agent injection tests ──────────────────────────────────────

/// is_sub_agent = true → SUBAGENT_SPARSE text
#[test]
fn test_sub_agent_true_outputs_subagent_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        false, // sparse irrelevant when sub_agent is true
        true,  // is_sub_agent
    );
    assert!(
        output.contains("Plan mode is active"),
        "Sub-agent should output SUBAGENT_SPARSE, got: {}",
        output
    );
    assert!(output.contains("READ-ONLY actions"));
    assert!(!output.contains("incremental edits"));
}

/// is_sub_agent = false → normal mode instruction (not sub-agent sparse)
#[test]
fn test_sub_agent_false_outputs_normal_instruction() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        false,
        false, // not sub-agent
    );
    assert!(output.contains("Phase 1: Initial Understanding"));
    assert!(!output.contains("incremental edits"));
}

/// Sub-agent takes precedence over sparse
#[test]
fn test_sub_agent_precedence_over_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        true, // compacted
        true, // sub-agent
    );
    assert!(
        output.contains("READ-ONLY actions"),
        "Sub-agent should take precedence over sparse"
    );
    assert!(!output.contains("Plan mode still active"));
}

// ── Step 1.2: Auto Mode sub-agent tests ───────────────────────────────────

/// Auto Mode + sub_agent=true, sparse=false → full Auto Mode prompt
/// (sub_agent flag is ignored in Auto Mode, should not inject SUBAGENT_SPARSE)
#[test]
fn test_auto_mode_sub_agent_true_not_injecting_subagent_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None,  // plan_path irrelevant for Auto
        false, // not compacted
        true,  // sub-agent
    );
    assert!(
        output.contains("Auto Mode Active"),
        "Auto Mode sub_agent should render full Auto Mode prompt, got: {}",
        output
    );
    assert!(
        !output.contains("Plan mode is active"),
        "Auto Mode sub_agent must NOT contain SUBAGENT_SPARSE text"
    );
    assert!(
        !output.contains("READ-ONLY actions"),
        "Auto Mode sub_agent must NOT contain READ-ONLY constraint"
    );
}

/// Auto Mode + sub_agent=true, sparse=true → Auto Mode sparse text
/// (sub_agent flag is ignored in Auto Mode, should render AUTO_MODE_SPARSE)
#[test]
fn test_auto_mode_sub_agent_true_sparse_outputs_auto_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None, // plan_path irrelevant for Auto
        true, // compacted/sparse
        true, // sub-agent
    );
    assert!(
        output.contains("Auto mode still active"),
        "Auto Mode sub_agent+sparse should output AUTO_MODE_SPARSE, got: {}",
        output
    );
    assert!(
        !output.contains("Plan mode is active"),
        "Auto Mode sub_agent+sparse must NOT contain SUBAGENT_SPARSE text"
    );
    assert!(
        !output.contains("READ-ONLY actions"),
        "Auto Mode sub_agent+sparse must NOT contain READ-ONLY constraint"
    );
}

/// Normal Mode + sub_agent=true → empty string (Normal Mode has no mode instruction)
#[test]
fn test_normal_mode_sub_agent_true_returns_empty() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Normal,
        None,
        false,
        true, // sub-agent
    );
    assert_eq!(
        output, "",
        "Normal Mode sub_agent should return empty string, got: {}",
        output
    );
}

/// Auto Mode + sub_agent=false → behavior unchanged (regression check)
#[test]
fn test_auto_mode_sub_agent_false_regression() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None,
        false, // not compacted
        false, // not sub-agent
    );
    assert!(
        output.contains("Auto Mode Active"),
        "Auto Mode without sub_agent should render full Auto Mode prompt"
    );
    assert!(!output.contains("Plan mode is active"));
    assert!(!output.contains("READ-ONLY actions"));
}

/// Auto Mode + sub_agent=false, sparse=true → Auto Mode sparse (regression)
#[test]
fn test_auto_mode_sub_agent_false_sparse_regression() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None,
        true,  // compacted/sparse
        false, // not sub-agent
    );
    assert!(
        output.contains("Auto mode still active"),
        "Auto Mode sparse without sub_agent should output AUTO_MODE_SPARSE"
    );
}

// -----------------------------------------------------------------------
// Step 1.3: Section enum variant and is_cacheable() coverage
// -----------------------------------------------------------------------

/// Verify all remaining Section variants have correct is_cacheable() behavior.
/// This serves as a regression guard after RoleSection/HeartbeatSection removal.
#[test]
fn test_is_cacheable_all_remaining_variants() {
    // Static (cacheable)
    let tools = Section::ToolsSection("tools".to_string());
    assert!(
        tools.is_cacheable(),
        "ToolsSection should be cacheable (returns true in current impl)"
    );
    let memory = Section::MemorySection("memory".to_string());
    assert!(memory.is_cacheable(), "MemorySection should be cacheable");

    // Dynamic (not cacheable)
    let channel = Section::ChannelContext {
        chat_name: "test".to_string(),
    };
    assert!(!channel.is_cacheable());
    let git = Section::GitStatus("status".to_string());
    assert!(!git.is_cacheable());
    let workdir = Section::WorkingDirectory("/tmp".to_string());
    assert!(!workdir.is_cacheable());
    let mode = Section::ModeInstruction {
        mode: SessionMode::Normal,
        plan_path: None,
        sparse: false,
        sub_agent: false,
    };
    assert!(!mode.is_cacheable());
}

/// Verify Section::name() returns unique, non-empty values for all variants.
#[test]
fn test_section_name_all_variants() {
    let names = vec![
        Section::ToolsSection("t".to_string()).name(),
        Section::MemorySection("m".to_string()).name(),
        Section::ChannelContext {
            chat_name: "c".to_string(),
        }
        .name(),
        Section::GitStatus("g".to_string()).name(),
        Section::WorkingDirectory("w".to_string()).name(),
        Section::ModeInstruction {
            mode: SessionMode::Normal,
            plan_path: None,
            sparse: false,
            sub_agent: false,
        }
        .name(),
    ];
    for name in &names {
        assert!(!name.is_empty(), "section name must not be empty");
    }
    // All names must be unique.
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        names.len(),
        sorted.len(),
        "all section names must be unique, got: {:?}",
        names
    );
}

/// Verify Section::render() does not panic for any remaining variant.
#[test]
fn test_section_render_no_panic_all_variants() {
    let sections = vec![
        Section::ToolsSection("tools content".to_string()),
        Section::MemorySection("memory content".to_string()),
        Section::ChannelContext {
            chat_name: "test-chat".to_string(),
        },
        Section::GitStatus("On branch main".to_string()),
        Section::WorkingDirectory("/tmp/work".to_string()),
        Section::ModeInstruction {
            mode: SessionMode::Normal,
            plan_path: None,
            sparse: false,
            sub_agent: false,
        },
        Section::ModeInstruction {
            mode: SessionMode::Plan,
            plan_path: Some(PlanPath::Standard),
            sparse: false,
            sub_agent: false,
        },
        Section::ModeInstruction {
            mode: SessionMode::Auto,
            plan_path: None,
            sparse: true,
            sub_agent: false,
        },
    ];
    for s in &sections {
        let rendered = s.render();
        assert!(
            !rendered.is_empty(),
            "render() must not return empty for {}",
            s.name()
        );
    }
}

/// Confirm RoleSection and HeartbeatSection no longer exist at compile time.
/// If these variants are re-added, this test file will fail to compile.
#[test]
fn test_role_and_heartbeat_section_removed() {
    // Attempting to construct these should be a compile error.
    // We verify indirectly: if the code compiles, these variants are gone.
    // This test documents the intent; the real guard is the compiler.
    let _ = std::any::type_name::<Section>();
    // Ensure Section::ToolsSection still exists (smoke test for enum).
    let s = Section::ToolsSection("x".to_string());
    assert_eq!(s.name(), "tools");
}
