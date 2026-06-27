//! Permission Engine - Core security component
//!
//! Runs as a separate OS process, evaluates access rules for agents.

pub mod actions;
pub mod approval;
pub mod approval_flow;
pub mod engine;
pub mod rules;
pub mod sandbox;
pub mod templates;

#[cfg(test)]
pub mod mock_session_lookup;

#[cfg(test)]
mod tests;

pub use engine::{
    glob_match, Action, Caller, CommandArgs, Defaults, Effect, MatchType, PermissionEngine,
    PermissionRequest, PermissionRequestBody, PermissionResponse, Rule, RuleSet, Subject,
    TemplateRef,
};
pub use rules::{validation, RuleBuilder, RuleBuilderError, RuleSetBuilder, RuleSetBuilderError};
