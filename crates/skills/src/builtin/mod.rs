//! Built-in skills - file_ops, git_ops, search, etc.

pub mod builder;
#[cfg(test)]
mod builder_tests;
pub mod create_workflow;
#[cfg(test)]
mod create_workflow_tests;
pub mod discovery;
#[cfg(test)]
mod discovery_test_helpers;
#[cfg(test)]
mod discovery_tests;
pub mod file_ops;
#[cfg(test)]
mod file_ops_tests;
pub mod git_ops;
#[cfg(test)]
mod git_ops_tests;
pub mod search;
#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod tests;

pub use builder::builtin_skills;
pub use create_workflow::WorkflowCreatorSkill;
pub use create_workflow::WorkflowDefinitionValidator;
pub use discovery::SkillDiscoverySkill;
pub use file_ops::FileOpsSkill;
pub use git_ops::GitOpsSkill;
pub use search::SearchSkill;
