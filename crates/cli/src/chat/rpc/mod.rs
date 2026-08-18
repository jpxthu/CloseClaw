//! Chat RPC sub-module — client, protocol, and (later) server for
//! CLI-to-daemon chat communication over Unix domain sockets.

pub mod client;
pub mod protocol;

pub use client::{chat_socket_path, ChatResponseStream, ChatRpcClient};
pub use protocol::{ChatRequest, ChatResponse};
