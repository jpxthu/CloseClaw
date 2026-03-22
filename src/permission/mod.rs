//! Permission Engine - Core security component
//!
//! Runs as a separate OS process, evaluates access rules for agents.

pub mod actions;
pub mod engine;
pub mod rules;
pub mod sandbox;
pub mod templates;

#[cfg(test)]
mod tests;

pub use engine::{
    PermissionEngine, PermissionRequest, PermissionRequestBody, PermissionResponse,
    Rule, RuleSet, Effect, Subject, MatchType, Action, CommandArgs, Defaults, Caller,
    TemplateRef, glob_match,
};
pub use rules::{
    RuleBuilder, RuleBuilderError, RuleSetBuilder, RuleSetBuilderError,
    validation,
};
