//! Tests for `DiskSkillRegistry::generate_listing_with_activated`.
//!
//! Covers the behavioral dimensions specified in Step 1.4 of the plan:
//! - Normal path: activated conditional skills appear in output
//! - User-invocable exemption: activated conditional skills bypass user_invocable
//! - Empty activation set: output matches `generate_listing_excluding_conditional`
//! - Error/boundary: nonexistent or non-conditional names in activated set
//! - Sorting consistency

use super::super::super::types::{SkillContext, SkillEffort, SkillManifest, SkillSource};
use super::super::super::DiskSkill;
use super::super::super::DiskSkillRegistry;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn skill(name: &str, source: SkillSource) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            when_to_use: String::new(),
            context: SkillContext::default(),
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable: true,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

fn skill_conditional(name: &str, source: SkillSource, paths: Vec<String>) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            when_to_use: String::new(),
            context: SkillContext::default(),
            effort: SkillEffort::default(),
            paths,
            user_invocable: true,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

fn skill_not_invocable_conditional(
    name: &str,
    source: SkillSource,
    paths: Vec<String>,
) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            when_to_use: String::new(),
            context: SkillContext::default(),
            effort: SkillEffort::default(),
            paths,
            user_invocable: false,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

fn skill_not_invocable(name: &str, source: SkillSource) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            when_to_use: String::new(),
            context: SkillContext::default(),
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable: false,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

// ---------------------------------------------------------------------------
// Dimension: Normal path — activated conditional skills in output
// ---------------------------------------------------------------------------

#[test]
fn test_activated_conditional_appears_in_listing() {
    let r = DiskSkillRegistry::new(vec![
        skill("base_skill", SkillSource::Bundled),
        skill_conditional("cond_skill", SkillSource::Bundled, vec!["**/*.rs".into()]),
    ]);
    let listing = r.generate_listing_with_activated(None, None, &["cond_skill".into()]);
    assert!(
        listing.contains("**cond_skill**"),
        "activated conditional skill must appear"
    );
    assert!(
        listing.contains("**base_skill**"),
        "base skill must still appear"
    );
}

#[test]
fn test_unactivated_conditional_excluded() {
    let r = DiskSkillRegistry::new(vec![
        skill("base_skill", SkillSource::Bundled),
        skill_conditional("cond_skill", SkillSource::Bundled, vec!["**/*.rs".into()]),
    ]);
    // Empty activated set → conditional skill excluded
    let listing = r.generate_listing_with_activated(None, None, &[]);
    assert!(listing.contains("**base_skill**"));
    assert!(
        !listing.contains("**cond_skill**"),
        "unactivated conditional must not appear"
    );
}

#[test]
fn test_multiple_activated_conditional_skills() {
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_conditional("cond_a", SkillSource::Bundled, vec!["**/*.rs".into()]),
        skill_conditional("cond_b", SkillSource::Global, vec!["**/*.md".into()]),
    ]);
    let listing =
        r.generate_listing_with_activated(None, None, &["cond_a".into(), "cond_b".into()]);
    assert!(listing.contains("**base**"));
    assert!(listing.contains("**cond_a**"));
    assert!(listing.contains("**cond_b**"));
    assert_eq!(listing.lines().count(), 3, "all three skills should appear");
}

#[test]
fn test_activated_conditional_contains_annotation() {
    let r = DiskSkillRegistry::new(vec![skill_conditional(
        "rs_skill",
        SkillSource::Bundled,
        vec!["**/*.rs".into()],
    )]);
    let listing = r.generate_listing_with_activated(None, None, &["rs_skill".into()]);
    assert!(
        listing.contains("auto-activates on: **/*.rs"),
        "activated conditional must include auto-activates annotation"
    );
}

#[test]
fn test_sorting_preserved_with_activated() {
    let r = DiskSkillRegistry::new(vec![
        skill("z_plain", SkillSource::Bundled),
        skill("a_plain", SkillSource::Bundled),
        skill_conditional("m_cond", SkillSource::Global, vec!["**/*.rs".into()]),
    ]);
    let listing = r.generate_listing_with_activated(None, None, &["m_cond".into()]);
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines.len(), 3);
    // Agent=0, Global=1, Bundled=2 — so m_cond (Global) before a_plain/z_plain (Bundled)
    let m_pos = listing.find("**m_cond**").unwrap();
    let a_pos = listing.find("**a_plain**").unwrap();
    let z_pos = listing.find("**z_plain**").unwrap();
    assert!(
        m_pos < a_pos,
        "Global-conditional (m_cond) must come before Bundled-plain (a_plain)"
    );
    assert!(a_pos < z_pos, "a_plain before z_plain within Bundled");
}

// ---------------------------------------------------------------------------
// Dimension: User-invocable exemption
// ---------------------------------------------------------------------------

#[test]
fn test_activated_conditional_not_invocable_still_included() {
    // Activated conditional skills are exempt from user_invocable filtering.
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_not_invocable_conditional(
            "hidden_cond",
            SkillSource::Bundled,
            vec!["**/*.rs".into()],
        ),
    ]);
    let listing = r.generate_listing_with_activated(None, None, &["hidden_cond".into()]);
    assert!(
        listing.contains("**hidden_cond**"),
        "activated conditional with user_invocable=false must still be included"
    );
    assert!(listing.contains("**base**"));
}

#[test]
fn test_unactivated_not_invocable_conditional_excluded() {
    // Without activation, user_invocable=false conditional skills are excluded.
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_not_invocable_conditional(
            "hidden_cond",
            SkillSource::Bundled,
            vec!["**/*.rs".into()],
        ),
    ]);
    let listing = r.generate_listing_with_activated(None, None, &[]);
    assert!(listing.contains("**base**"));
    assert!(
        !listing.contains("**hidden_cond**"),
        "unactivated non-invocable conditional must not appear"
    );
}

#[test]
fn test_not_invocable_non_conditional_normal_rules_apply() {
    // A non-conditional skill with user_invocable=false is excluded
    // regardless of being in the activated set (activation only
    // exempts conditional skills).
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_not_invocable("hidden_plain", SkillSource::Bundled),
    ]);
    let listing = r.generate_listing_with_activated(None, None, &["hidden_plain".into()]);
    assert!(listing.contains("**base**"));
    assert!(
        !listing.contains("**hidden_plain**"),
        "non-conditional skill with user_invocable=false must not be exempted by activation"
    );
}

// ---------------------------------------------------------------------------
// Dimension: Empty activation set — regresses to base listing
// ---------------------------------------------------------------------------

#[test]
fn test_empty_activated_set_matches_excluding_conditional() {
    let r = DiskSkillRegistry::new(vec![
        skill("alpha", SkillSource::Bundled),
        skill("beta", SkillSource::Global),
        skill_conditional("gamma", SkillSource::Agent, vec!["**/*.rs".into()]),
    ]);
    let with_activated = r.generate_listing_with_activated(None, None, &[]);
    let base = r.generate_listing_excluding_conditional(None, None);
    assert_eq!(
        with_activated, base,
        "empty activated set must produce identical output to base listing"
    );
}

#[test]
fn test_empty_activated_set_all_conditional_registry_returns_empty() {
    let r = DiskSkillRegistry::new(vec![skill_conditional(
        "only_cond",
        SkillSource::Bundled,
        vec!["**/*.rs".into()],
    )]);
    let listing = r.generate_listing_with_activated(None, None, &[]);
    assert!(
        listing.is_empty(),
        "empty activated set with all-conditional registry must be empty"
    );
}

// ---------------------------------------------------------------------------
// Dimension: Error/boundary — nonexistent skill names in activated set
// ---------------------------------------------------------------------------

#[test]
fn test_nonexistent_skill_in_activated_set_ignored() {
    let r = DiskSkillRegistry::new(vec![skill("real_skill", SkillSource::Bundled)]);
    let listing = r.generate_listing_with_activated(
        None,
        None,
        &["deleted_skill".into(), "another_gone".into()],
    );
    assert!(
        listing.contains("**real_skill**"),
        "real skill must still appear"
    );
    assert!(
        !listing.contains("deleted_skill"),
        "nonexistent skill must not appear"
    );
    assert_eq!(listing.lines().count(), 1);
}

#[test]
fn test_nonexistent_skill_with_real_activated() {
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_conditional("real_cond", SkillSource::Bundled, vec!["**/*.rs".into()]),
    ]);
    let listing =
        r.generate_listing_with_activated(None, None, &["real_cond".into(), "gone_skill".into()]);
    assert!(listing.contains("**base**"));
    assert!(listing.contains("**real_cond**"));
    assert_eq!(
        listing.lines().count(),
        2,
        "only real skills should appear; nonexistent silently ignored"
    );
}

#[test]
fn test_nonexistent_only_in_activated_returns_base_only() {
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_conditional("cond", SkillSource::Bundled, vec!["**/*.rs".into()]),
    ]);
    let listing = r.generate_listing_with_activated(None, None, &["nonexistent".into()]);
    assert!(listing.contains("**base**"));
    assert!(
        !listing.contains("**cond**"),
        "unactivated conditional must not appear"
    );
}

// ---------------------------------------------------------------------------
// Dimension: Non-conditional skill in activated set — normal user_invocable rules
// ---------------------------------------------------------------------------

#[test]
fn test_non_conditional_in_activated_set_follows_normal_rules() {
    // A non-conditional skill with user_invocable=true in the activated set
    // is included because it's user_invocable, not because of activation.
    let r = DiskSkillRegistry::new(vec![skill("always_visible", SkillSource::Bundled)]);
    let listing = r.generate_listing_with_activated(None, None, &["always_visible".into()]);
    assert!(listing.contains("**always_visible**"));
}

#[test]
fn test_non_conditional_not_invocable_not_exempt_by_activated() {
    // Activation does NOT exempt non-conditional skills from user_invocable filter.
    let r = DiskSkillRegistry::new(vec![skill_not_invocable("hidden", SkillSource::Bundled)]);
    let listing = r.generate_listing_with_activated(None, None, &["hidden".into()]);
    assert!(
        listing.is_empty(),
        "non-conditional with user_invocable=false must remain excluded even when activated"
    );
}

// ---------------------------------------------------------------------------
// Dimension: Whitelist interaction
// ---------------------------------------------------------------------------

#[test]
fn test_activated_conditional_respects_whitelist() {
    let r = DiskSkillRegistry::new(vec![
        skill("alpha", SkillSource::Bundled),
        skill_conditional("beta_cond", SkillSource::Bundled, vec!["**/*.rs".into()]),
        skill("gamma", SkillSource::Bundled),
    ]);
    // Whitelist only allows alpha and beta_cond
    let listing = r.generate_listing_with_activated(
        None,
        Some(&["alpha".into(), "beta_cond".into()]),
        &["beta_cond".into()],
    );
    assert!(listing.contains("**alpha**"));
    assert!(listing.contains("**beta_cond**"));
    assert!(!listing.contains("**gamma**"));
}

#[test]
fn test_activated_conditional_not_in_whitelist_excluded() {
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_conditional("cond", SkillSource::Bundled, vec!["**/*.rs".into()]),
    ]);
    // Whitelist does NOT include "cond", so it's excluded even though activated
    let listing = r.generate_listing_with_activated(None, Some(&["base".into()]), &["cond".into()]);
    assert!(listing.contains("**base**"));
    assert!(
        !listing.contains("**cond**"),
        "conditional not in whitelist must be excluded even when activated"
    );
}

#[test]
fn test_wildcard_whitelist_includes_activated_conditional() {
    let r = DiskSkillRegistry::new(vec![
        skill("base", SkillSource::Bundled),
        skill_conditional("cond", SkillSource::Bundled, vec!["**/*.rs".into()]),
    ]);
    let listing = r.generate_listing_with_activated(None, Some(&["*".into()]), &["cond".into()]);
    assert!(listing.contains("**base**"));
    assert!(listing.contains("**cond**"));
}

// ---------------------------------------------------------------------------
// Dimension: Empty registry
// ---------------------------------------------------------------------------

#[test]
fn test_empty_registry_returns_empty() {
    let r = DiskSkillRegistry::new(vec![]);
    let listing = r.generate_listing_with_activated(None, None, &["any_skill".into()]);
    assert!(listing.is_empty());
}

// ---------------------------------------------------------------------------
// Dimension: Source priority ordering with activated skills
// ---------------------------------------------------------------------------

#[test]
fn test_activated_conditional_respects_source_priority() {
    let r = DiskSkillRegistry::new(vec![
        skill("base_bundled", SkillSource::Bundled),
        skill_conditional("cond_project", SkillSource::Project, vec!["**/*.rs".into()]),
        skill_conditional("cond_global", SkillSource::Global, vec!["**/*.md".into()]),
    ]);
    let listing = r.generate_listing_with_activated(
        None,
        None,
        &["cond_project".into(), "cond_global".into()],
    );
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines.len(), 3);
    // Project=0, Global=1, Bundled=2
    let project_pos = listing.find("**cond_project**").unwrap();
    let global_pos = listing.find("**cond_global**").unwrap();
    let bundled_pos = listing.find("**base_bundled**").unwrap();
    assert!(
        project_pos < global_pos,
        "Project (cond_project) before Global (cond_global)"
    );
    assert!(
        global_pos < bundled_pos,
        "Global (cond_global) before Bundled (base_bundled)"
    );
}
