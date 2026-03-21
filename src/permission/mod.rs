//! Permission Engine - Core security component
//!
//! Runs as a separate OS process, evaluates access rules for agents.

pub mod actions;
pub mod engine;
pub mod rules;
pub mod sandbox;

pub use engine::{
    PermissionEngine, PermissionRequest, PermissionResponse, Rule, RuleSet,
    Effect, Subject, MatchType, Action, CommandArgs, Defaults, glob_match,
};
pub use rules::{
    RuleBuilder, RuleBuilderError, RuleSetBuilder, RuleSetBuilderError,
    validation,
};
