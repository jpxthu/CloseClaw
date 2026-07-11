//! Spawn module — child session creation, validation, and tracking.
//!
//! Provides `SpawnController` for validating spawn requests and
//! shared types (`ChildSessionInfo`, `SpawnMode`, `SpawnValidationResult`)
//! used across session and gateway crates.

pub mod controller;
pub mod error;
pub mod tree;
pub mod types;

pub use controller::SpawnContext;
pub use controller::SpawnController;
pub use error::SpawnError;
pub use tree::SpawnTree;
pub use types::ChildSessionInfo;
pub use types::SpawnMode;
pub use types::SpawnValidationResult;
