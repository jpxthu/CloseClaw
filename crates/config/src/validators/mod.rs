//! Validators for hot-reloaded config sections.
//!
//! Each validator performs structural checks and lightweight business
//! validation (field presence, range, format) on the parsed JSON value.
//! Deep cross-section validation (e.g., credentials reference resolution)
//! belongs in the startup path via Provider `validate()` methods.

mod accounts;
mod channels;
mod cross_ref;
mod helpers;
mod memory;
mod models;
mod registry;
mod sections;
mod session;
mod tools;

pub use accounts::validate_accounts;
pub use channels::{validate_channels, validate_channels_with_refs};
pub use cross_ref::{CredentialProviderSet, CrossRefData};
pub use memory::validate_memory;
pub use models::{validate_models, validate_models_with_refs};
pub use registry::for_section;
pub use sections::{
    validate_agents, validate_credentials, validate_gateway, validate_media, validate_plugins,
    validate_skills, validate_system,
};
pub use session::validate_session;
pub use tools::validate_tools;

// Items referenced by sibling submodules via `use super::{...}` keep their
// module-scope names here.
pub(crate) use helpers::{ensure_object, type_name};
pub(crate) use sections::validate_non_negative_field;

#[cfg(test)]
#[path = "../validators_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../validators_cron_tests.rs"]
mod validators_cron_tests;

#[cfg(test)]
#[path = "../validators_session_archive_audit_tests.rs"]
mod validators_session_archive_audit_tests;

#[cfg(test)]
#[path = "../validators_session_compact_tests.rs"]
mod validators_session_compact_tests;

#[cfg(test)]
#[path = "../validators_memory_tests.rs"]
mod validators_memory_tests;

#[cfg(test)]
#[path = "../validators_step_1_7_tests.rs"]
mod validators_step_1_7_tests;
