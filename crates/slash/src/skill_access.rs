//! Skill registry access ports for the slash crate.
//!
//! [`DiskSkillAccess`] and [`BuiltinSkillAccess`] are slash-owned
//! abstractions over the two skill registries, narrowed to the exact
//! call surface of [`crate::skill_handler::SkillSlashHandler`]: list
//! user-invocable names, load a disk skill's body + directory, and
//! execute a builtin skill by name. `${SKILL_DIR}` substitution stays
//! in the handler (data comes from the port), and error `Display`
//! strings of the mirror types match the skills-side originals
//! verbatim so reply texts are byte-identical.
//!
//! The composition root (daemon) supplies the production
//! implementations wrapping `closeclaw_skills` registries; tests may
//! use the thin wrappers in [`real_access`] (skills is a
//! dev-dependency of this crate).

use std::path::PathBuf;
use thiserror::Error;

/// A disk skill's instruction body plus its directory, as needed by
/// the slash handler for `${SKILL_DIR}` substitution.
#[derive(Debug, Clone)]
pub struct DiskSkillBody {
    /// Instruction text loaded from the skill's `SKILL.md`
    /// (frontmatter stripped).
    pub body: String,
    /// Absolute path to the skill directory.
    pub skill_dir: PathBuf,
}

/// Slash-owned port over the disk skill registry.
///
/// Implementations must be `Send + Sync` (handlers are shared behind
/// `Arc`).
pub trait DiskSkillAccess: Send + Sync {
    /// Names of all user-invocable disk skills.
    fn user_invocable_names(&self) -> Vec<String>;

    /// Load a skill's body and directory by exact name.
    ///
    /// `None` means no disk skill with that name exists (the caller
    /// falls back to the builtin registry). `Some(Err)` means the
    /// skill exists but its body could not be loaded (e.g. `SKILL.md`
    /// deleted after registration); the [`std::io::Error`] `Display`
    /// is surfaced verbatim in the reply.
    fn load_by_name(&self, name: &str) -> Option<Result<DiskSkillBody, std::io::Error>>;
}

/// Errors that can occur when executing a builtin skill (slash-owned
/// mirror).
///
/// `Display` strings must stay identical to the skills-side originals —
/// they are surfaced verbatim in replies
/// (`技能 "{name}" 执行失败: {e}`).
#[derive(Debug, Clone, Error)]
pub enum SkillExecuteError {
    /// Skill lookup failed inside the skill implementation.
    #[error("Skill '{0}' not found")]
    NotFound(String),

    /// Skill execution failed.
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    /// Invalid arguments passed to the skill.
    #[error("Invalid arguments: {0}")]
    InvalidArgs(String),
}

/// Slash-owned port over the builtin skill registry.
#[async_trait::async_trait]
pub trait BuiltinSkillAccess: Send + Sync {
    /// Names of all user-invocable builtin skills.
    async fn user_invocable_names(&self) -> Vec<String>;

    /// Execute a builtin skill by exact name.
    ///
    /// `None` means no builtin skill with that name exists (the caller
    /// replies 未知技能). `Some(Err)` means the skill exists but
    /// execution failed; the error `Display` is surfaced verbatim in
    /// the reply.
    async fn execute_by_name(&self, name: &str) -> Option<Result<String, SkillExecuteError>>;
}

/// Test-support thin port implementations wrapping the real
/// `closeclaw_skills` registries (skills is a dev-dependency of this
/// crate).
///
/// Only compiled for the slash crate's own tests; the production
/// implementation lives in the daemon composition root.
#[cfg(test)]
pub(crate) mod real_access {
    use std::sync::Arc;

    use closeclaw_skills::disk::DiskSkillRegistry;
    use closeclaw_skills::{BuiltinSkillRegistry, SkillError};

    use super::{BuiltinSkillAccess, DiskSkillAccess, DiskSkillBody, SkillExecuteError};

    /// Map a skills-side execute error to the slash-side mirror,
    /// preserving the payload (and therefore the `Display` output)
    /// verbatim.
    pub(crate) fn execute_error_to_slash(e: SkillError) -> SkillExecuteError {
        match e {
            SkillError::NotFound(name) => SkillExecuteError::NotFound(name),
            SkillError::ExecutionFailed(msg) => SkillExecuteError::ExecutionFailed(msg),
            SkillError::InvalidArgs(msg) => SkillExecuteError::InvalidArgs(msg),
        }
    }

    /// Thin wrapper around the real [`DiskSkillRegistry`].
    pub(crate) struct RealDiskSkillAccess(pub Arc<DiskSkillRegistry>);

    impl DiskSkillAccess for RealDiskSkillAccess {
        fn user_invocable_names(&self) -> Vec<String> {
            self.0.user_invocable_names()
        }

        fn load_by_name(&self, name: &str) -> Option<Result<DiskSkillBody, std::io::Error>> {
            self.0.get(name).map(|skill| {
                let skill_dir = skill.skill_dir.clone();
                skill
                    .load_body()
                    .map(|body| DiskSkillBody { body, skill_dir })
            })
        }
    }

    /// Thin wrapper around the real [`BuiltinSkillRegistry`].
    pub(crate) struct RealBuiltinSkillAccess(pub Arc<BuiltinSkillRegistry>);

    #[async_trait::async_trait]
    impl BuiltinSkillAccess for RealBuiltinSkillAccess {
        async fn user_invocable_names(&self) -> Vec<String> {
            self.0.user_invocable_names().await
        }

        async fn execute_by_name(&self, name: &str) -> Option<Result<String, SkillExecuteError>> {
            let skill = self.0.get(name).await?;
            Some(skill.execute(None).await.map_err(execute_error_to_slash))
        }
    }
}
