//! Tests for `SkillListingProviderWrapper::generate_listing_with_activated`.
//!
//! Extracted from `bridge.rs` to stay within the 1000-line limit.

use super::*;
use async_trait::async_trait;
use closeclaw_common::SkillListingProvider;
use closeclaw_skills::disk::types::{DiskSkill, SkillSource};
use closeclaw_skills::DiskSkillRegistry;
use closeclaw_skills::SkillManifest;
use std::path::PathBuf;
use std::sync::Arc;

fn run_with_runtime<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    std::thread::scope(|s| {
        s.spawn(|| {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            let _guard = rt.enter();
            f();
        })
        .join()
        .expect("test thread panicked")
    })
}

fn make_disk_skill(
    source: SkillSource,
    name: &str,
    user_invocable: bool,
    paths: Vec<String>,
) -> DiskSkill {
    DiskSkill {
        source,
        manifest: closeclaw_skills::disk::types::SkillManifest {
            name: name.to_string(),
            description: format!("disk skill {name}"),
            when_to_use: String::new(),
            context: Default::default(),
            effort: Default::default(),
            paths,
            user_invocable,
        },
        readme_path: PathBuf::from(format!("/tmp/{name}/SKILL.md")),
        skill_dir: PathBuf::from(format!("/tmp/{name}")),
    }
}

fn make_builtin_skill(
    name: &str,
    user_invocable: bool,
    paths: Vec<String>,
) -> Arc<dyn closeclaw_skills::Skill> {
    struct MockBuiltin {
        manifest: SkillManifest,
    }

    #[async_trait]
    impl closeclaw_skills::Skill for MockBuiltin {
        fn manifest(&self) -> SkillManifest {
            self.manifest.clone()
        }
        fn body(&self) -> &str {
            "mock body"
        }
    }

    Arc::new(MockBuiltin {
        manifest: SkillManifest {
            name: name.to_string(),
            description: format!("builtin skill {}", name),
            when_to_use: String::new(),
            context: Default::default(),
            effort: Default::default(),
            paths,
            user_invocable,
        },
    })
}

fn make_disk_registry(skills: Vec<DiskSkill>) -> DiskSkillRegistry {
    closeclaw_skills::DiskSkillRegistry::new(skills)
}

fn make_wrapper(
    disk: DiskSkillRegistry,
    builtin: Arc<closeclaw_skills::BuiltinSkillRegistry>,
) -> SkillListingProviderWrapper {
    SkillListingProviderWrapper::new(Arc::new(std::sync::RwLock::new(Some(disk))), builtin)
}

// ------------------------------------------------------------------
// generate_listing_with_activated — normal path
// ------------------------------------------------------------------

#[test]
fn test_with_activated_includes_activated_conditional_disk() {
    run_with_runtime(|| {
        let disk = make_disk_registry(vec![
            make_disk_skill(SkillSource::Bundled, "base_skill", true, vec![]),
            make_disk_skill(
                SkillSource::Bundled,
                "cond_skill",
                true,
                vec!["**/*.rs".to_string()],
            ),
        ]);
        let builtin = Arc::new(closeclaw_skills::BuiltinSkillRegistry::new());
        let wrapper = make_wrapper(disk, builtin);

        let listing =
            wrapper.generate_listing_with_activated(None, None, &["cond_skill".to_string()]);
        assert!(listing.contains("**base_skill**"));
        assert!(listing.contains("**cond_skill**"));
    });
}

#[test]
fn test_with_activated_includes_activated_conditional_builtin() {
    run_with_runtime(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let builtin = rt.block_on(async {
            Arc::new(
                closeclaw_skills::BuiltinSkillRegistry::from_skills(vec![
                    make_builtin_skill("base_b", true, vec![]),
                    make_builtin_skill("cond_b", true, vec!["**/*.toml".to_string()]),
                ])
                .await,
            )
        });
        let disk = make_disk_registry(vec![]);
        let wrapper = make_wrapper(disk, builtin);

        let listing = wrapper.generate_listing_with_activated(None, None, &["cond_b".to_string()]);
        assert!(listing.contains("**base_b**"));
        assert!(listing.contains("**cond_b**"));
    });
}

#[test]
fn test_with_activated_excludes_unactivated_conditional() {
    run_with_runtime(|| {
        let disk = make_disk_registry(vec![
            make_disk_skill(SkillSource::Bundled, "base", true, vec![]),
            make_disk_skill(
                SkillSource::Bundled,
                "cond",
                true,
                vec!["**/*.rs".to_string()],
            ),
        ]);
        let builtin = Arc::new(closeclaw_skills::BuiltinSkillRegistry::new());
        let wrapper = make_wrapper(disk, builtin);

        let listing = wrapper.generate_listing_with_activated(None, None, &[]);
        assert!(listing.contains("**base**"));
        assert!(!listing.contains("**cond**"));
    });
}

#[test]
fn test_with_activated_empty_matches_excluding_conditional() {
    run_with_runtime(|| {
        let disk = make_disk_registry(vec![
            make_disk_skill(SkillSource::Bundled, "alpha", true, vec![]),
            make_disk_skill(
                SkillSource::Global,
                "beta_cond",
                true,
                vec!["**/*.md".to_string()],
            ),
        ]);
        let builtin = Arc::new(closeclaw_skills::BuiltinSkillRegistry::new());
        let wrapper = make_wrapper(disk, builtin);

        let with_activated = wrapper.generate_listing_with_activated(None, None, &[]);
        let base = wrapper.generate_listing_excluding_conditional(None, None);
        assert_eq!(
            with_activated, base,
            "empty activated set must match base listing"
        );
    });
}

#[test]
fn test_with_activated_nonexistent_skill_ignored() {
    run_with_runtime(|| {
        let disk = make_disk_registry(vec![make_disk_skill(
            SkillSource::Bundled,
            "real",
            true,
            vec![],
        )]);
        let builtin = Arc::new(closeclaw_skills::BuiltinSkillRegistry::new());
        let wrapper = make_wrapper(disk, builtin);

        let listing = wrapper.generate_listing_with_activated(None, None, &["deleted".to_string()]);
        assert!(listing.contains("**real**"));
        assert!(!listing.contains("deleted"));
    });
}

#[test]
fn test_with_activated_cross_registry_merge() {
    run_with_runtime(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let builtin = rt.block_on(async {
            Arc::new(
                closeclaw_skills::BuiltinSkillRegistry::from_skills(vec![
                    make_builtin_skill("builtin_only", true, vec![]),
                    make_builtin_skill("shared", true, vec![]),
                ])
                .await,
            )
        });
        let disk = make_disk_registry(vec![make_disk_skill(
            SkillSource::Project,
            "shared",
            true,
            vec![],
        )]);
        let wrapper = make_wrapper(disk, builtin);

        let listing = wrapper.generate_listing_with_activated(None, None, &[]);
        let count = listing.matches("**shared**").count();
        assert_eq!(count, 1, "shared must appear exactly once");
        assert!(listing.contains("**builtin_only**"));
    });
}

#[test]
fn test_with_activated_empty_registries() {
    run_with_runtime(|| {
        let disk = make_disk_registry(vec![]);
        let builtin = Arc::new(closeclaw_skills::BuiltinSkillRegistry::new());
        let wrapper = make_wrapper(disk, builtin);

        let listing = wrapper.generate_listing_with_activated(None, None, &["any".to_string()]);
        assert!(listing.is_empty());
    });
}
