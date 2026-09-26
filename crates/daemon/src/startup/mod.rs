//! Startup orchestration: component dependency declarations and data structures.
//!
//! Defines [`ComponentId`] to identify each daemon component and [`ComponentDeps`]
//! to declare startup dependencies. The topological sort engine (see
//! [`topo_sort_layers`]) consumes these declarations to derive the deterministic
//! initialization order.

mod component;
mod graph;
mod phase;

pub use component::{ComponentDeps, ComponentEntry, ComponentId, Foundation, Service};
pub use graph::{all_component_entries, topo_sort_layers, StartupError};
pub use phase::{validate_startup_layers, StartupPhase};

#[cfg(test)]
mod startup_tests;
