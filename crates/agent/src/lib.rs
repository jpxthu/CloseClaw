//! Agent module - pure configuration layer for agent definitions.

pub mod config;
pub mod registry;
pub mod spawn;

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod spawn_tests;

#[cfg(test)]
#[path = "spawn_budget_tests.rs"]
mod spawn_budget_tests;

#[cfg(test)]
#[path = "spawn_permission_tests.rs"]
mod spawn_permission_tests;
