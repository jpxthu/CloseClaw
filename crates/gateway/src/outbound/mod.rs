//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses through the unified
//! [`IMPlugin`] registry.

pub mod batch;
pub mod chain_prep;
pub mod streaming;
pub mod types;

pub use types::{OutboundMeta, SendOutboundIds, SendOutcome, StreamResult};
