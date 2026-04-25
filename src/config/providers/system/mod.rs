//! System JSON ConfigProvider
//!
//! Loads and validates the system section of openclaw.json.
//! Covers: wizard, update, meta, messages, commands, session, cron,
//!         hooks, browser, auth (profiles only — no apiKey).

mod system_core;
pub use system_core::*;

#[cfg(test)]
mod tests;
