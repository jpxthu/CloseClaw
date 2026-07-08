//! Permission Engine - Core security component
//!
//! Runs as a separate OS process, evaluates access rules for agents.

pub mod engine_chain;
pub mod engine_check;
pub mod engine_eval;
pub mod engine_helpers;
pub mod engine_matching;
pub mod engine_risk;
pub mod engine_spawn;
pub mod engine_types;
pub mod engine_workspace;
pub mod rejection_log;

pub use engine_eval::PermissionEngine;
pub use engine_matching::{action_matches_request, glob_match};
pub use engine_risk::RiskLevel;
pub use engine_types::{
    Action, Caller, CommandArgs, Defaults, Effect, MatchType, PermissionRequest,
    PermissionRequestBody, PermissionResponse, Rule, RuleSet, Subject, TemplateRef,
};
pub use rejection_log::{build_rejection_log, FileRejectionLogger, RejectionLog, RejectionLogger};

#[cfg(test)]
mod engine_tests;

#[cfg(test)]
mod engine_workspace_tests;

#[cfg(test)]
mod engine_owner_tests;

#[cfg(test)]
mod engine_two_phase_tests;

#[cfg(test)]
mod engine_types_tests;

#[cfg(test)]
mod engine_chain_tests;

#[cfg(test)]
mod engine_spawn_tests;

#[cfg(test)]
mod engine_plan_mode_tests;

#[cfg(test)]
mod engine_rejection_log_tests;
