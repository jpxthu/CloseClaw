//! Outbound message routing for the Gateway.
//!
//! Handles rendering and dispatching agent responses through the unified
//! [`IMPlugin`](closeclaw_common::im_plugin::IMPlugin) registry.

pub(crate) mod batch;
pub(crate) mod batch_dispatch;
pub(crate) mod batch_fallback;
pub(crate) mod chain_prep;
#[cfg(test)]
mod chain_prep_tests;
pub(crate) mod streaming;
pub(crate) mod streaming_finalize;
pub(crate) mod streaming_incremental;
pub(crate) mod types;

pub use types::{OutboundMeta, SendOutboundIds, SendOutcome, StreamResult};
