//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses through the unified
//! [`IMPlugin`] registry.

pub(crate) mod batch;
pub(crate) mod chain_prep;
pub(crate) mod streaming;
pub(crate) mod types;

pub use types::{OutboundMeta, SendOutboundIds, SendOutcome, StreamResult};
