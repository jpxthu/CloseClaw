//! Bridge module tests — `SkillListingProviderWrapper` integration.
//!
//! Extracted from `bridge.rs` to stay within the 1000-line limit.

use super::test_support::{
    make_builtin, make_builtin_blocking, make_disk_registry, make_disk_skill, make_wrapper,
    run_with_runtime,
};
use super::*;
use closeclaw_common::AgentSkillsQuery;
use closeclaw_common::SkillListingProvider;
use closeclaw_skills::disk::types::SkillSource;
use closeclaw_skills::BuiltinSkillRegistry;
use closeclaw_skills::DiskSkillRegistry;
use closeclaw_skills::ScanConfig;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Shared builtin skill name used across whitelist/sorting tests.
const ALPHA: &str = "alpha";

// ------------------------------------------------------------------
// Test helpers
// ------------------------------------------------------------------

/// Mock `AgentSkillsQuery` that returns a fixed whitelist for each agent.
struct MockAgentSkillsQuery {
    /// agent_id → skills whitelist
    skills: HashMap<String, Vec<String>>,
}

impl MockAgentSkillsQuery {
    fn new(skills: HashMap<String, Vec<String>>) -> Self {
        Self { skills }
    }
}

impl AgentSkillsQuery for MockAgentSkillsQuery {
    fn get_agent_skills(&self, agent_id: &str) -> Option<Vec<String>> {
        self.skills.get(agent_id).cloned()
    }
}

// ------------------------------------------------------------------
// Test 1: Agent whitelist filters builtin skills
// ------------------------------------------------------------------

#[test]
fn test_whitelist_filters_builtin_skills() {
    run_with_runtime(|| {
        let builtin = make_builtin_blocking(vec![
            (ALPHA, true, vec![]),
            ("beta", true, vec![]),
            ("gamma", true, vec![]),
        ]);
        let disk = make_disk_registry(vec![]);
        let wrapper = make_wrapper(disk, builtin);

        // No whitelist → all builtin skills shown
        let listing = wrapper.generate_listing(None, None);
        assert!(listing.contains("alpha"));
        assert!(listing.contains("beta"));
        assert!(listing.contains("gamma"));

        // Whitelist with only beta → only beta shown
        let listing = wrapper.generate_listing(None, Some(&["beta".to_string()]));
        assert!(!listing.contains("alpha"));
        assert!(listing.contains("beta"));
        assert!(!listing.contains("gamma"));

        // Wildcard "*" → all shown
        let listing = wrapper.generate_listing(None, Some(&["*".to_string()]));
        assert!(listing.contains("alpha"));
        assert!(listing.contains("beta"));
        assert!(listing.contains("gamma"));
    });
}

// ------------------------------------------------------------------
// Test 2: Agent skills query fallback for builtin filtering
// ------------------------------------------------------------------

#[test]
fn test_builtin_filtered_via_agent_skills_query() {
    run_with_runtime(|| {
        let builtin = make_builtin_blocking(vec![
            (ALPHA, true, vec![]),
            ("beta", true, vec![]),
            ("gamma", true, vec![]),
        ]);

        // Simulate agent_skills_query returning whitelist ["alpha", "gamma"]
        // by injecting the query into the disk registry.
        let mut disk_reg = DiskSkillRegistry::new(vec![]);
        disk_reg.set_agent_skills_query(Arc::new(MockAgentSkillsQuery::new(HashMap::from([(
            "agent1".to_string(),
            vec!["alpha".to_string(), "gamma".to_string()],
        )]))));
        let wrapper = make_wrapper(disk_reg, builtin);

        // agent_skills=None → unified resolver falls back to query
        let listing = wrapper.generate_listing(Some("agent1"), None);
        assert!(listing.contains("alpha"));
        assert!(!listing.contains("beta"));
        assert!(listing.contains("gamma"));
    });
}

// ------------------------------------------------------------------
// Test 3: Disk skills override builtin skills with same name
// ------------------------------------------------------------------

#[test]
fn test_disk_overrides_builtin() {
    run_with_runtime(|| {
        let builtin = make_builtin_blocking(vec![
            ("shared_skill", true, vec![]),
            ("other_skill", true, vec![]),
        ]);
        let disk = make_disk_registry(vec![make_disk_skill(
            SkillSource::Project,
            "shared_skill",
            true,
            vec![],
        )]);

        let wrapper = make_wrapper(disk, builtin);
        let listing = wrapper.generate_listing(None, None);

        // shared_skill appears once (disk version wins)
        let count = listing.matches("- **shared_skill**: ").count();
        assert_eq!(count, 1, "shared_skill should appear exactly once");
        // other_skill (builtin only) still appears
        assert!(listing.contains("other_skill"));
    });
}

// ------------------------------------------------------------------
// Test 4: Cross-registry sorting by source priority then name
// ------------------------------------------------------------------

#[test]
fn test_cross_registry_sorting() {
    run_with_runtime(|| {
        let builtin = make_builtin_blocking(vec![("b_skill", true, vec![])]);
        // Disk: Agent-priority "z_skill", Project-priority "a_skill"
        let disk = make_disk_registry(vec![
            make_disk_skill(SkillSource::Agent, "z_skill", true, vec![]),
            make_disk_skill(SkillSource::Project, "a_skill", true, vec![]),
        ]);

        let wrapper = make_wrapper(disk, builtin);
        let listing = wrapper.generate_listing(None, None);

        // Expected order: a_skill (Project=0), z_skill (Agent=1), b_skill (Bundled=4)
        let a_pos = listing.find("a_skill").unwrap();
        let z_pos = listing.find("z_skill").unwrap();
        let b_pos = listing.find("b_skill").unwrap();
        assert!(
            a_pos < z_pos && z_pos < b_pos,
            "expected a_skill < z_skill < b_skill but got a={a_pos} z={z_pos} b={b_pos}"
        );
    });
}

// ------------------------------------------------------------------
// Test 5: Conditional skill exclusion
// ------------------------------------------------------------------

#[test]
fn test_conditional_exclusion() {
    run_with_runtime(|| {
        let builtin = make_builtin_blocking(vec![
            ("regular", true, vec![]),
            ("conditional", true, vec!["**/*.rs".to_string()]),
        ]);
        let disk = make_disk_registry(vec![]);
        let wrapper = make_wrapper(disk, builtin);

        // generate_listing_excluding_conditional → conditional excluded
        let listing = wrapper.generate_listing_excluding_conditional(None, None);
        assert!(listing.contains("regular"));
        assert!(!listing.contains("conditional"));
    });
}

// ------------------------------------------------------------------
// Test 6: SP rebuild path — listing includes activated conditionals,
//         excluding does not
// ------------------------------------------------------------------

#[test]
fn test_sp_rebuild_path() {
    run_with_runtime(|| {
        let builtin = make_builtin_blocking(vec![
            ("always", true, vec![]),
            ("conditional", true, vec!["**/*.rs".to_string()]),
        ]);
        let disk = make_disk_registry(vec![]);
        let wrapper = make_wrapper(disk, builtin);

        // Full listing (generate_listing) includes conditional
        let full = wrapper.generate_listing(None, None);
        assert!(full.contains("always"));
        assert!(full.contains("conditional"));

        // Excluding listing does not include conditional
        let base = wrapper.generate_listing_excluding_conditional(None, None);
        assert!(base.contains("always"));
        assert!(!base.contains("conditional"));
    });
}

// ------------------------------------------------------------------
// Test 7: Whitelist filters both disk and builtin skills
// ------------------------------------------------------------------

#[test]
fn test_whitelist_filters_disk_and_builtin() {
    run_with_runtime(|| {
        let builtin = make_builtin_blocking(vec![
            ("builtin_only", true, vec![]),
            ("shared", true, vec![]),
        ]);
        let disk = make_disk_registry(vec![
            make_disk_skill(SkillSource::Global, "disk_only", true, vec![]),
            make_disk_skill(SkillSource::Global, "shared", true, vec![]),
        ]);
        let wrapper = make_wrapper(disk, builtin);

        // Whitelist ["shared"] → only shared appears (from disk)
        let listing = wrapper.generate_listing(None, Some(&["shared".to_string()]));
        assert!(listing.contains("shared"));
        assert!(!listing.contains("disk_only"));
        assert!(!listing.contains("builtin_only"));
    });
}

// ------------------------------------------------------------------
// Test 8: find_conditional_matches from merged registries
// ------------------------------------------------------------------

#[test]
fn test_find_conditional_matches_merged() {
    run_with_runtime(|| {
        let builtin = make_builtin_blocking(vec![
            ("rust_skill", true, vec!["**/*.rs".to_string()]),
            ("txt_skill", true, vec!["**/*.txt".to_string()]),
        ]);
        let disk = make_disk_registry(vec![]);
        let wrapper = make_wrapper(disk, builtin);

        let matches = wrapper.find_conditional_matches(&[PathBuf::from("src/main.rs")]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "rust_skill");
    });
}

// ------------------------------------------------------------------
// Test 10: SkillListingProviderWrapper::rescan() integration
// ------------------------------------------------------------------

#[test]
fn test_wrapper_rescan_picks_up_new_disk_skill() {
    run_with_runtime(|| {
        // Create a temp directory with an initial skill
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("existing-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: existing\nuser-invocable: true\n---\n# Existing\n",
        )
        .unwrap();

        // Build disk registry with scan_config pointing at temp dir
        let mut disk_reg = DiskSkillRegistry::new(vec![]);
        disk_reg.set_scan_config(ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        });
        disk_reg.rescan(); // initial scan

        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let wrapper = make_wrapper(disk_reg, builtin);

        // Listing should contain the existing skill
        let listing = wrapper.generate_listing(None, None);
        assert!(listing.contains("existing-skill"));

        // Add a new skill to disk
        let new_skill_dir = temp.path().join("new-skill");
        std::fs::create_dir_all(&new_skill_dir).unwrap();
        std::fs::write(
            new_skill_dir.join("SKILL.md"),
            "---\ndescription: new skill\nuser-invocable: true\n---\n# New\n",
        )
        .unwrap();

        // After rescan, listing should reflect the new skill
        wrapper.rescan();
        let listing = wrapper.generate_listing(None, None);
        assert!(listing.contains("new-skill"));
        assert!(listing.contains("existing-skill"));
    });
}

// ------------------------------------------------------------------
// Test 11: empty registries produce empty listing
// ------------------------------------------------------------------

#[test]
fn test_empty_registries() {
    run_with_runtime(|| {
        let disk = make_disk_registry(vec![]);
        let builtin = Arc::new(BuiltinSkillRegistry::new());

        let wrapper = make_wrapper(disk, builtin);
        assert!(wrapper.generate_listing(None, None).is_empty());
        assert!(wrapper
            .generate_listing_excluding_conditional(None, None)
            .is_empty());
    });
}

// ------------------------------------------------------------------
// Dimension: Fingerprint changes when disk skills are modified
// ------------------------------------------------------------------

#[test]
fn test_fingerprint_changes_after_rescan_picks_up_new_skill() {
    run_with_runtime(|| {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("skill-a");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: a\nuser-invocable: true\n---\n# A\n",
        )
        .unwrap();

        let mut disk_reg = DiskSkillRegistry::new(vec![]);
        disk_reg.set_scan_config(ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        });
        disk_reg.rescan();

        let disk = Arc::new(std::sync::RwLock::new(Some(disk_reg)));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let wrapper = SkillListingProviderWrapper::new(disk.clone(), builtin);

        let fp_before = wrapper.fingerprint();
        let listing_before = wrapper.generate_listing(None, None);
        assert!(listing_before.contains("skill-a"));

        // Add new skill
        let new_dir = temp.path().join("skill-b");
        std::fs::create_dir_all(&new_dir).unwrap();
        std::fs::write(
            new_dir.join("SKILL.md"),
            "---\ndescription: b\nuser-invocable: true\n---\n# B\n",
        )
        .unwrap();

        wrapper.rescan();
        let fp_after = wrapper.fingerprint();
        let listing_after = wrapper.generate_listing(None, None);

        assert_ne!(fp_before, fp_after, "fingerprint must change after rescan");
        assert!(listing_after.contains("skill-a"));
        assert!(listing_after.contains("skill-b"));
    });
}

// ------------------------------------------------------------------
// Dimension: Fingerprint stable with degraded disk (no scan_config)
// ------------------------------------------------------------------

#[test]
fn test_fingerprint_stable_when_no_scan_config() {
    run_with_runtime(|| {
        let disk = make_disk_registry(vec![make_disk_skill(
            SkillSource::Bundled,
            "existing",
            true,
            vec![],
        )]);
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        let wrapper = make_wrapper(disk, builtin);

        // No scan_config set → fingerprint uses count fallback
        let fp = wrapper.fingerprint();
        assert!(
            fp.starts_with("disk:count:"),
            "should use count fallback: {fp}"
        );
        // Multiple calls return the same value
        assert_eq!(fp, wrapper.fingerprint());
    });
}

// Regression: sync trait entry points driven from an async context
// must not panic with "Cannot start a runtime from within a runtime".

#[tokio::test(flavor = "multi_thread")]
async fn test_sync_entry_points_safe_in_async_context() {
    let builtin = make_builtin(vec![
        (ALPHA, true, vec![]),
        ("conditional", true, vec!["**/*.rs".to_string()]),
    ])
    .await;
    let disk = make_disk_registry(vec![]);
    let wrapper = make_wrapper(disk, builtin);

    // ① generate_listing → collect_builtin_listings
    let listing = wrapper.generate_listing(None, None);
    assert!(listing.contains("alpha"));

    // ② generate_listing_with_activated → collect_builtin_activated_entries
    let listing = wrapper.generate_listing_with_activated(None, None, &["conditional".to_string()]);
    assert!(listing.contains("conditional"));

    // ③ find_conditional_matches → merged_conditional_matches
    let matches = wrapper.find_conditional_matches(&[PathBuf::from("src/main.rs")]);
    assert_eq!(matches.len(), 1, "conditional skill should match");

    // ④ fingerprint → combined_fingerprint
    let fp = wrapper.fingerprint();
    assert!(fp.starts_with("disk:"), "unexpected fingerprint: {fp}");
}
