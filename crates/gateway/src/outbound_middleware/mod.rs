//! Built-in outbound middleware implementations.
//!
//! - [`audit`]: records audit logs for every outbound message.
//! - [`rate_limit`]: session-level sliding-window rate limiting.

pub mod audit;
pub mod rate_limit;

#[cfg(test)]
pub(crate) mod audit_tests;
#[cfg(test)]
pub(crate) mod rate_limit_tests;
