//! Built-in outbound middleware implementations and their execution.
//!
//! - [`audit`]: records audit logs for every outbound message.
//! - [`rate_limit`]: session-level sliding-window rate limiting.
//! - [`runner`]: Gateway-owned middleware chain / pre-flight execution
//!   (common-trait only — no concrete processor-chain dependency).

pub mod audit;
pub mod rate_limit;
pub(crate) mod runner;

#[cfg(test)]
pub(crate) mod audit_tests;
#[cfg(test)]
pub(crate) mod rate_limit_tests;
