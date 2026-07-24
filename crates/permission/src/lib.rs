//! Permission Engine - Core security component
//!
//! Runs as a separate OS process, evaluates access rules for agents.

pub mod actions;
pub mod approval;
pub mod approval_flow;
pub mod engine;
pub mod rules;
pub mod sandbox;
pub mod skill_wrapper;
pub mod templates;
pub mod user_registry;
pub mod whitelist;

#[cfg(test)]
pub mod user_registry_tests;

#[cfg(test)]
pub mod mock_session_lookup;

#[cfg(test)]
mod tests;

pub use engine::{
    build_rejection_log, glob_match, is_config_file_path, Action, Caller, CommandArgs, Defaults,
    Effect, FileRejectionLogger, MatchType, PermissionEngine, PermissionRequest,
    PermissionRequestBody, PermissionResponse, RejectionLog, RejectionLogger, Rule, RuleSet,
    Subject, TemplateRef,
};
pub use rules::{validation, RuleBuilder, RuleBuilderError, RuleSetBuilder, RuleSetBuilderError};
pub use user_registry::{RegistryError, UserRegistry};

#[cfg(test)]
pub mod permission_op_tests;
