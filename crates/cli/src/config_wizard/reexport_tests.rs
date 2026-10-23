//! Re-export completeness regression tests (issue #3952 Step 1.4).
//!
//! Compile-time verification that every externally-depended symbol is still
//! reachable via the `crate::config_wizard::` path after the module split.
//! If a re-export in `mod.rs` is removed or renamed, these checks fail to
//! compile — no runtime assertions needed.

use std::path::Path;

/// Symbols used by external callers and test glob imports
/// (admin/config.rs:182-218, admin/config_tests.rs:898-904, tests.rs glob).
#[test]
fn test_config_wizard_reexports_compile() {
    // admin/config.rs: `run_wizard` (async; path resolution proves the
    // re-export) and `write_wizard_config`.
    let _ = crate::config_wizard::run_wizard;
    let _write_wizard_config: fn(&crate::config_wizard::WizardOutput) -> anyhow::Result<()> =
        crate::config_wizard::write_wizard_config;

    // admin/config_tests.rs: `write_wizard_config_to` / `WizardOutput`.
    let _write_wizard_config_to: fn(
        &crate::config_wizard::WizardOutput,
        &Path,
    ) -> anyhow::Result<()> = crate::config_wizard::write_wizard_config_to;
    let _wizard_output: fn() -> crate::config_wizard::WizardOutput = || unreachable!();

    // Test glob deps (tests.rs `use super::*;` / `use super::{...}`).
    let _parse_model_selection: fn(&str, usize) -> anyhow::Result<Vec<usize>> =
        crate::config_wizard::parse_model_selection;
    let _compute_default_selection: fn(
        &[closeclaw_llm::model_info::ModelInfo],
        &std::collections::HashSet<String>,
    ) -> String = crate::config_wizard::compute_default_selection;
    let _format_discovery_source_suffix: fn(closeclaw_llm::DiscoverySource) -> &'static str =
        crate::config_wizard::format_discovery_source_suffix;
    let _wizard_context: fn() -> crate::config_wizard::WizardContext = || unreachable!();
    let _providers: &'static [crate::config_wizard::ProviderInfo] = crate::config_wizard::PROVIDERS;
    let _wizard_state: crate::config_wizard::WizardState =
        crate::config_wizard::WizardState::SelectProvider;

    // fetch.rs re-export surfaced via `pub use fetch::*;`.
    let _ = crate::config_wizard::knowledge_fallback;

    // pub(crate) re-exports consumed across the crate.
    let _ensure_master_agent: fn(&Path, &Path) -> anyhow::Result<()> =
        crate::config_wizard::ensure_master_agent;
}
