//! Permission rule helpers and builders
//!
//! Provides builder patterns and validation for [`Rule`], [`Subject`], and [`RuleSet`] types.

mod builder;
mod ruleset_builder;
#[cfg(test)]
mod tests;
pub mod validation;

pub use builder::{RuleBuilder, RuleBuilderError};
pub use ruleset_builder::{RuleSetBuilder, RuleSetBuilderError};
