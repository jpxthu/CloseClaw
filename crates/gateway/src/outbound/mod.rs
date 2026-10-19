//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses through the unified
//! [`IMPlugin`] registry.

pub mod pipeline;

pub use pipeline::{OutboundMeta, SendOutboundIds, SendOutcome, StreamResult};
