//! Admin RPC sub-module — client, protocol, and server for
//! CLI-to-daemon communication over Unix domain sockets.

pub mod client;
pub mod protocol;
pub mod server;

pub use client::{admin_socket_path, AdminClient};
pub use protocol::{AdminRequest, AdminResponse};
pub use server::{AdminContext, AdminServer};

#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod server_tests;
