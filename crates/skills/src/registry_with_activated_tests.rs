//! Tests for `BuiltinSkillRegistry::generate_listing_with_activated`.

use super::*;
use crate::disk::types::SkillEffort;
use std::sync::Arc;

struct MockSkill {
    manifest: SkillManifest,
}

impl MockSkill {
    fn with_manifest(_name: &str, manifest: SkillManifest) -> Self {
        Self { manifest }
    }
}

#[async_trait]
impl Skill for MockSkill {
    fn manifest(&self) -> SkillManifest {
        self.manifest.clone()
    }

    fn body(&self) -> &str {
        "mock body"
    }
}

#[tokio::test]
async fn test_with_activated_includes_activated_conditional() {
    let registry = BuiltinSkillRegistry::from_skills(vec![
        Arc::new(MockSkill::with_manifest(
            "base_skill",
            SkillManifest {
                name: "base_skill".into(),
                description: "mock skill base_skill".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec![],
                user_invocable: true,
            },
        )),
        Arc::new(MockSkill::with_manifest(
            "cond_skill",
            SkillManifest {
                name: "cond_skill".into(),
                description: "mock skill cond_skill".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec!["**/*.rs".to_string()],
                user_invocable: true,
            },
        )),
    ])
    .await;
    let listing = registry
        .generate_listing_with_activated(&["cond_skill".to_string()])
        .await;
    assert!(listing.contains("**base_skill**"));
    assert!(listing.contains("**cond_skill**"));
}

#[tokio::test]
async fn test_with_activated_excludes_unactivated_conditional() {
    let registry = BuiltinSkillRegistry::from_skills(vec![
        Arc::new(MockSkill::with_manifest(
            "base_skill",
            SkillManifest {
                name: "base_skill".into(),
                description: "mock skill base_skill".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec![],
                user_invocable: true,
            },
        )),
        Arc::new(MockSkill::with_manifest(
            "cond_skill",
            SkillManifest {
                name: "cond_skill".into(),
                description: "mock skill cond_skill".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec!["**/*.rs".to_string()],
                user_invocable: true,
            },
        )),
    ])
    .await;
    let listing = registry.generate_listing_with_activated(&[]).await;
    assert!(listing.contains("**base_skill**"));
    assert!(!listing.contains("**cond_skill**"));
}

#[tokio::test]
async fn test_with_activated_empty_matches_excluding_conditional() {
    let registry = BuiltinSkillRegistry::from_skills(vec![
        Arc::new(MockSkill::with_manifest(
            "alpha",
            SkillManifest {
                name: "alpha".into(),
                description: "mock skill alpha".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec![],
                user_invocable: true,
            },
        )),
        Arc::new(MockSkill::with_manifest(
            "cond_beta",
            SkillManifest {
                name: "cond_beta".into(),
                description: "mock skill cond_beta".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec!["**/*.md".to_string()],
                user_invocable: true,
            },
        )),
    ])
    .await;
    let with_activated = registry.generate_listing_with_activated(&[]).await;
    let base = registry.generate_listing_excluding_conditional().await;
    assert_eq!(
        with_activated, base,
        "empty activated set must match base listing"
    );
}

#[tokio::test]
async fn test_with_activated_nonexistent_skill_ignored() {
    let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_manifest(
        "real_skill",
        SkillManifest {
            name: "real_skill".into(),
            description: "mock skill real_skill".into(),
            when_to_use: String::new(),
            context: crate::disk::types::SkillContext::default(),
            effort: SkillEffort::Unknown,
            paths: vec![],
            user_invocable: true,
        },
    ))])
    .await;
    let listing = registry
        .generate_listing_with_activated(&["nonexistent".to_string()])
        .await;
    assert!(listing.contains("**real_skill**"));
    assert!(!listing.contains("nonexistent"));
}

#[tokio::test]
async fn test_with_activated_not_invocable_conditional_excluded_without_activation() {
    let registry = BuiltinSkillRegistry::from_skills(vec![
        Arc::new(MockSkill::with_manifest(
            "base",
            SkillManifest {
                name: "base".into(),
                description: "mock skill base".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec![],
                user_invocable: true,
            },
        )),
        Arc::new(MockSkill::with_manifest(
            "hidden_cond",
            SkillManifest {
                name: "hidden_cond".into(),
                description: "mock skill hidden_cond".into(),
                when_to_use: String::new(),
                context: crate::disk::types::SkillContext::default(),
                effort: SkillEffort::Unknown,
                paths: vec!["**/*.rs".to_string()],
                user_invocable: false,
            },
        )),
    ])
    .await;
    let listing = registry.generate_listing_with_activated(&[]).await;
    assert!(listing.contains("**base**"));
    assert!(!listing.contains("**hidden_cond**"));
}

#[tokio::test]
async fn test_with_activated_annotation_present() {
    let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_manifest(
        "rs_skill",
        SkillManifest {
            name: "rs_skill".into(),
            description: "mock skill rs_skill".into(),
            when_to_use: String::new(),
            context: crate::disk::types::SkillContext::default(),
            effort: SkillEffort::Unknown,
            paths: vec!["**/*.rs".to_string()],
            user_invocable: true,
        },
    ))])
    .await;
    let listing = registry
        .generate_listing_with_activated(&["rs_skill".to_string()])
        .await;
    assert!(listing.contains("auto-activates on: **/*.rs"));
}

#[tokio::test]
async fn test_with_activated_empty_registry() {
    let registry = BuiltinSkillRegistry::new();
    let listing = registry
        .generate_listing_with_activated(&["any".to_string()])
        .await;
    assert!(listing.is_empty());
}

#[tokio::test]
async fn test_with_activated_non_conditional_in_activated_follows_normal_rules() {
    let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_manifest(
        "hidden_plain",
        SkillManifest {
            name: "hidden_plain".into(),
            description: "mock skill hidden_plain".into(),
            when_to_use: String::new(),
            context: crate::disk::types::SkillContext::default(),
            effort: SkillEffort::Unknown,
            paths: vec![],
            user_invocable: false,
        },
    ))])
    .await;
    let listing = registry
        .generate_listing_with_activated(&["hidden_plain".to_string()])
        .await;
    assert!(
        listing.is_empty(),
        "non-conditional user_invocable=false must not be exempted by activation"
    );
}
