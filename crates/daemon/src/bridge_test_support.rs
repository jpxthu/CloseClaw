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
use closeclaw_skills::ScanConfig;
use closeclaw_skills::Skill;
use closeclaw_skills::SkillManifest;
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;

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

/// Helper: build a `BuiltinSkillRegistry` from `(name, user_invocable, paths)`.
pub(super) async fn make_builtin(
    skills: Vec<(&str, bool, Vec<String>)>,
) -> Arc<BuiltinSkillRegistry> {
    Arc::new(
        BuiltinSkillRegistry::from_skills(
            skills
                .into_iter()
                .map(|(n, u, p)| make_builtin_skill(n, u, p))
                .collect(),
        )
        .await,
    )
}

/// Helper: blocking variant of [`make_builtin`] for synchronous test
/// bodies: builds a dedicated runtime and blocks on the async builder.
pub(super) fn make_builtin_blocking(
    skills: Vec<(&str, bool, Vec<String>)>,
) -> Arc<BuiltinSkillRegistry> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(make_builtin(skills))
}

/// Helper: create a `DiskSkillRegistry` with the given skills.
pub(super) fn make_disk_registry(skills: Vec<DiskSkill>) -> DiskSkillRegistry {
    DiskSkillRegistry::new(skills)
}

/// Helper: write a single-level skill directory with a SKILL.md
/// frontmatter into the existing directory `root`, named `name`.
///
/// Uses single-level `std::fs::create_dir` (parent must exist), matching
/// the loader.rs test pattern and the test-audit baseline's stance on
/// `create_dir_all`.
pub(super) fn write_skill_dir(root: &Path, name: &str) {
    let skill_dir = root.join(name);
    std::fs::create_dir(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\ndescription: {name}\nuser-invocable: true\n---\n# {name}\n"),
    )
    .unwrap();
}

/// Helper: build a `DiskSkillRegistry` whose `ScanConfig` points at
/// `global_dir`, with the initial rescan already performed.
pub(super) fn make_scan_dir_registry(global_dir: &Path) -> DiskSkillRegistry {
    let mut disk_reg = DiskSkillRegistry::new(vec![]);
    disk_reg.set_scan_config(ScanConfig {
        global_dir: Some(global_dir.to_path_buf()),
        ..Default::default()
    });
    disk_reg.rescan(); // initial scan
    disk_reg
}

/// Helper: wrap a `DiskSkillRegistry` in the shared
/// `Arc<RwLock<Option<...>>>` slot used by the bridge wrappers.
pub(super) fn make_slot(disk: DiskSkillRegistry) -> Arc<RwLock<Option<DiskSkillRegistry>>> {
    Arc::new(RwLock::new(Some(disk)))
}

/// Helper: build a scan-dir registry via [`make_scan_dir_registry`] and
/// wrap it via [`make_slot`].
pub(super) fn make_scan_dir_slot(global_dir: &Path) -> Arc<RwLock<Option<DiskSkillRegistry>>> {
    make_slot(make_scan_dir_registry(global_dir))
}

/// Helper: create a `SkillListingProviderWrapper` from disk and builtin
/// registries.
pub(super) fn make_wrapper(
    disk: DiskSkillRegistry,
    builtin: Arc<BuiltinSkillRegistry>,
) -> SkillListingProviderWrapper {
    SkillListingProviderWrapper::new(make_slot(disk), builtin)
}
