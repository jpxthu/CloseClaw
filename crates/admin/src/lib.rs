//! Admin RPC module for CLI-to-daemon communication.
//!
//! Provides the admin server (daemon-side) and client (CLI-side)
//! for managing agents and skills via a Unix domain socket.

pub mod client;
pub mod protocol;
pub mod server;

pub use client::AdminClient;
pub use protocol::{AdminRequest, AdminResponse};
pub use server::AdminServer;

#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod server_tests;
