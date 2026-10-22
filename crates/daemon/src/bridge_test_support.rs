//! Shared test support for the bridge test modules (`tests` and
//! `activated_tests`).
//!
//! Extracted from byte-identical per-file duplicates so each helper
//! has exactly one definition (#3923).

use super::SkillListingProviderWrapper;
use async_trait::async_trait;
use closeclaw_skills::disk::types::{DiskSkill, SkillSource};
use closeclaw_skills::BuiltinSkillRegistry;
use closeclaw_skills::DiskSkillRegistry;
use closeclaw_skills::Skill;
use closeclaw_skills::SkillManifest;
use std::sync::Arc;

/// Run a closure on a dedicated thread with a tokio runtime context
/// established via `enter()`. `Handle::current()` inside the wrapper
/// helpers resolves via this guard (the actual await runs on a
/// scoped thread joined before returning — see `block_on_join_thread`).
pub(super) fn run_with_runtime<F>(f: F)
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

/// Helper: build a `DiskSkill` with the given source and name.
pub(super) fn make_disk_skill(
    source: SkillSource,
    name: &str,
    user_invocable: bool,
    paths: Vec<String>,
) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.to_string(),
            description: format!("disk skill {name}"),
            when_to_use: String::new(),
            context: Default::default(),
            effort: Default::default(),
            paths,
            user_invocable,
        },
        readme_path: std::env::temp_dir().join(name).join("SKILL.md"),
        skill_dir: std::env::temp_dir().join(name),
    }
}

/// Helper: build a mock builtin skill with the given name and meta.
pub(super) fn make_builtin_skill(
    name: &str,
    user_invocable: bool,
    paths: Vec<String>,
) -> Arc<dyn Skill> {
    struct MockBuiltin {
        manifest: SkillManifest,
    }

    #[async_trait]
    impl Skill for MockBuiltin {
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

/// Helper: create a `DiskSkillRegistry` with the given skills.
pub(super) fn make_disk_registry(skills: Vec<DiskSkill>) -> DiskSkillRegistry {
    DiskSkillRegistry::new(skills)
}

/// Helper: create a `SkillListingProviderWrapper` from disk and builtin
/// registries.
pub(super) fn make_wrapper(
    disk: DiskSkillRegistry,
    builtin: Arc<BuiltinSkillRegistry>,
) -> SkillListingProviderWrapper {
    SkillListingProviderWrapper::new(Arc::new(std::sync::RwLock::new(Some(disk))), builtin)
}
