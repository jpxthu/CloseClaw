//! Agent communication permission checks.
//!
//! Re-exports from `closeclaw_common::communication` for convenient
//! access within the spawn module.

pub use closeclaw_common::communication::{
    check_communication_allowed, CommunicationCheckResult, CommunicationConfig, CommunicationError,
};
