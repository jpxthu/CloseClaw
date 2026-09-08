//! Spawn module — child session creation, validation, and tracking.
//!
//! Provides `SpawnController` for validating spawn requests,
//! `create_child_conversation_session` for building child sessions,
//! shared types (`ChildSessionInfo`, `ChildSessionStatus`, `SpawnMode`, `SpawnValidationResult`),
//! and the `SpawnCreationContext` trait for dependency injection.

pub mod communication;
pub mod context;
pub mod controller;
pub mod creation;
pub mod tree;
pub mod types;

pub use communication::{
    check_communication_allowed, CommunicationCheckResult, CommunicationConfig, CommunicationError,
};
pub use context::SpawnCreationContext;
pub use controller::SpawnContext;
pub use controller::SpawnController;
pub use creation::{
    build_spawn_context, create_child_conversation_session, ChildSessionCreated,
    ChildSessionCreationParams,
};
pub use tree::SpawnTree;
pub use types::ChildSessionInfo;
pub use types::ChildSessionStatus;
pub use types::SpawnMode;

#[cfg(test)]
#[path = "controller_tests.rs"]
mod controller_tests;

#[cfg(test)]
#[path = "creation_tests.rs"]
mod creation_tests;

#[cfg(test)]
#[path = "spawn_role_timing_tests.rs"]
mod spawn_role_timing_tests;

#[cfg(test)]
mod tree_tests;
