//! Command sandbox routing for BashTool.
//!
//! Provides [`CommandSandbox`] for routing commands to sandboxed or
//! unsandboxed execution based on permission checks.

pub mod command_sandbox;

pub use command_sandbox::CommandSandbox;
