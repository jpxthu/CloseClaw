//! Skill registry access ports for the tools crate.
//!
//! [`DiskSkillAccess`] and [`BuiltinSkillAccess`] are tools-owned
//! abstractions over the two skill registries, narrowed to the exact
//! call surface of [`crate::builtin::skill_tool::SkillTool`]: load a
//! disk skill's body + directory by name, and execute a builtin skill
//! by name with optional JSON args. `${SKILL_DIR}` substitution stays
//! in the tool (data comes from the port), and the error `Display`
//! strings of the mirror type match the skills-side originals
//! verbatim so `ToolCallError` messages are byte-identical.
//!
//! The composition root (daemon) supplies the production
//! implementations wrapping `closeclaw_skills` registries; tests may
//! use the thin wrappers in the `real_access` test helper (skills is
//! a dev-dependency of this crate).

use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use thiserror::Error;

/// A disk skill's instruction body plus its directory, as needed by
/// the tool for `${SKILL_DIR}` substitution.
#[derive(Debug, Clone)]
pub struct DiskSkillData {
    /// Instruction text loaded from the skill's `SKILL.md`
    /// (frontmatter stripped).
    pub body: String,
    /// Absolute path to the skill directory.
    pub skill_dir: PathBuf,
}

/// Tools-owned port over the disk skill registry.
///
/// Implementations must be `Send + Sync` (tools are shared behind
/// `Arc`).
pub trait DiskSkillAccess: Send + Sync {
    /// Load a skill's body and directory by exact name.
    ///
    /// `None` means no disk skill with that name exists (the tool
    /// falls back to the builtin registry). `Some(Err)` means the
    /// skill exists but its body could not be loaded (e.g. `SKILL.md`
    /// deleted after registration); the [`std::io::Error`] `Display`
    /// is surfaced verbatim in the `ToolCallError` message.
    fn load_by_name(&self, name: &str) -> Option<Result<DiskSkillData, std::io::Error>>;
}

/// Errors that can occur when executing a builtin skill (tools-owned
/// mirror).
///
/// `Display` strings must stay identical to the skills-side originals
/// — they are surfaced verbatim in `ToolCallError::ExecutionFailed`
/// (`skill execution failed: {e}`).
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

/// Tools-owned port over the builtin skill registry.
#[async_trait]
pub trait BuiltinSkillAccess: Send + Sync {
    /// Execute a builtin skill by exact name, forwarding the caller's
    /// optional `args` payload.
    ///
    /// `None` means no builtin skill with that name exists (the tool
    /// reports `ToolCallError::NotFound`). `Some(Err)` means the skill
    /// exists but execution failed; the error `Display` is surfaced
    /// verbatim in the `ToolCallError` message.
    async fn execute_by_name(
        &self,
        name: &str,
        args: Option<Value>,
    ) -> Option<Result<String, SkillExecuteError>>;
}

/// Test-support thin port implementations wrapping the real
/// `closeclaw_skills` registries (skills is a dev-dependency of this
/// crate).
///
/// Only compiled for the tools crate's own tests; the production
/// implementation lives in the daemon composition root.
#[cfg(test)]
pub(crate) mod real_access {
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Arc;

    use closeclaw_skills::disk::DiskSkillRegistry;
    use closeclaw_skills::{BuiltinSkillRegistry, SkillError};

    use super::{BuiltinSkillAccess, DiskSkillAccess, DiskSkillData, SkillExecuteError};

    /// Map a skills-side execute error to the tools-side mirror,
    /// preserving the payload (and therefore the `Display` output)
    /// verbatim.
    pub(crate) fn execute_error_to_mirror(e: SkillError) -> SkillExecuteError {
        match e {
            SkillError::NotFound(name) => SkillExecuteError::NotFound(name),
            SkillError::ExecutionFailed(msg) => SkillExecuteError::ExecutionFailed(msg),
            SkillError::InvalidArgs(msg) => SkillExecuteError::InvalidArgs(msg),
        }
    }

    /// Thin wrapper around the real [`DiskSkillRegistry`].
    pub(crate) struct RealDiskSkillAccess(pub Arc<DiskSkillRegistry>);

    impl DiskSkillAccess for RealDiskSkillAccess {
        fn load_by_name(&self, name: &str) -> Option<Result<DiskSkillData, std::io::Error>> {
            self.0.get(name).map(|skill| {
                let skill_dir = skill.skill_dir.clone();
                skill
                    .load_body()
                    .map(|body| DiskSkillData { body, skill_dir })
            })
        }
    }

    /// Thin wrapper around the real [`BuiltinSkillRegistry`].
    pub(crate) struct RealBuiltinSkillAccess(pub Arc<BuiltinSkillRegistry>);

    #[async_trait]
    impl BuiltinSkillAccess for RealBuiltinSkillAccess {
        async fn execute_by_name(
            &self,
            name: &str,
            args: Option<Value>,
        ) -> Option<Result<String, SkillExecuteError>> {
            let skill = self.0.get(name).await?;
            Some(skill.execute(args).await.map_err(execute_error_to_mirror))
        }
    }
}
