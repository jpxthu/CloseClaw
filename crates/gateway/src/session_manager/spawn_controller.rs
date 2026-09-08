//! SpawnController re-export from the session crate.
//!
//! After Step 1.2, the canonical `SpawnController` lives in
//! `closeclaw_session::spawn::controller`. This module re-exports it
//! for backward compatibility so that `closeclaw_gateway::SpawnController`
//! continues to resolve to the same type.
//!
//! The session crate's `SpawnController` implements `SpawnValidator`
//! (from `closeclaw_session::spawn_validation`), so it can be used
//! directly as `Arc<dyn SpawnValidator>` in the tools layer.

pub use closeclaw_session::spawn::controller::SpawnController;
