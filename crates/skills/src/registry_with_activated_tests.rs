//! Tests for `BuiltinSkillRegistry::generate_listing_with_activated`.

use super::*;
use crate::disk::types::SkillEffort;
use std::sync::Arc;

struct MockSkill {
    name: String,
    meta: SkillListingMeta,
}

impl MockSkill {
    fn with_meta(name: &str, meta: SkillListingMeta) -> Self {
        Self {
            name: name.to_string(),
            meta,
        }
    }
}

#[async_trait]
impl Skill for MockSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: self.name.clone(),
            description: format!("mock skill {}", self.name),
            when_to_use: self.meta.when_to_use.clone(),
            context: crate::disk::types::SkillContext::default(),
            effort: self.meta.effort,
            paths: self.meta.paths.clone(),
            user_invocable: self.meta.user_invocable,
        }
    }

    fn body(&self) -> &str {
        "mock body"
    }

    fn listing_meta(&self) -> SkillListingMeta {
        self.meta.clone()
    }
}

#[tokio::test]
async fn test_with_activated_includes_activated_conditional() {
    let registry = BuiltinSkillRegistry::from_skills(vec![
        Arc::new(MockSkill::with_meta(
            "base_skill",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: true,
                paths: vec![],
                effort: SkillEffort::Unknown,
            },
        )),
        Arc::new(MockSkill::with_meta(
            "cond_skill",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: true,
                paths: vec!["**/*.rs".to_string()],
                effort: SkillEffort::Unknown,
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
        Arc::new(MockSkill::with_meta(
            "base_skill",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: true,
                paths: vec![],
                effort: SkillEffort::Unknown,
            },
        )),
        Arc::new(MockSkill::with_meta(
            "cond_skill",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: true,
                paths: vec!["**/*.rs".to_string()],
                effort: SkillEffort::Unknown,
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
        Arc::new(MockSkill::with_meta(
            "alpha",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: true,
                paths: vec![],
                effort: SkillEffort::Unknown,
            },
        )),
        Arc::new(MockSkill::with_meta(
            "cond_beta",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: true,
                paths: vec!["**/*.md".to_string()],
                effort: SkillEffort::Unknown,
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
    let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_meta(
        "real_skill",
        SkillListingMeta {
            when_to_use: String::new(),
            user_invocable: true,
            paths: vec![],
            effort: SkillEffort::Unknown,
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
        Arc::new(MockSkill::with_meta(
            "base",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: true,
                paths: vec![],
                effort: SkillEffort::Unknown,
            },
        )),
        Arc::new(MockSkill::with_meta(
            "hidden_cond",
            SkillListingMeta {
                when_to_use: String::new(),
                user_invocable: false,
                paths: vec!["**/*.rs".to_string()],
                effort: SkillEffort::Unknown,
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
    let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_meta(
        "rs_skill",
        SkillListingMeta {
            when_to_use: String::new(),
            user_invocable: true,
            paths: vec!["**/*.rs".to_string()],
            effort: SkillEffort::Unknown,
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
    let registry = BuiltinSkillRegistry::from_skills(vec![Arc::new(MockSkill::with_meta(
        "hidden_plain",
        SkillListingMeta {
            when_to_use: String::new(),
            user_invocable: false,
            paths: vec![],
            effort: SkillEffort::Unknown,
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
