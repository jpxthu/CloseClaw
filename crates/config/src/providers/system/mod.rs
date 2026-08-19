//! System JSON ConfigProvider
//!
//! Loads and validates the system section of openclaw.json.
//! Covers: wizard, update, meta, messages, commands, session, cron,
//!         hooks, browser, auth (profiles only — no apiKey).

mod system_core;
pub use system_core::*;

// Re-export SkillsConfig from its new home in the skills provider
// so existing imports via `providers::system::SkillsConfig` keep working.
pub use super::skills::SkillsConfig;

#[cfg(test)]
mod tests;
