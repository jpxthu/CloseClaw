//! Shared test helpers for daemon tests.
//!
//! Duplicated from root crate's `common/test_helpers.rs` because the daemon
//! crate cannot depend on the root crate (circular dependency).

pub mod config;
pub mod dispatch;
pub mod fakes;
pub mod lifecycle;
pub mod scheduler;
pub mod shutdown;

pub use config::*;
pub(crate) use dispatch::*;
pub use fakes::*;
pub(crate) use lifecycle::*;
pub use scheduler::*;
pub use shutdown::*;
