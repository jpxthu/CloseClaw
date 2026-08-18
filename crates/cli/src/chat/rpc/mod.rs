//! Chat RPC protocol types for CLI ↔ daemon communication.
//!
//! Uses length-prefixed JSON frames over a Unix domain socket:
//! ```text
//! [4-byte big-endian length (u32)][JSON frame bytes]
//! ```

pub mod protocol;

pub use protocol::{ChatRequest, ChatResponse};
