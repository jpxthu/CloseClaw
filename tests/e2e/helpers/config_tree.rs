//! Minimal config-tree scaffolding shared by e2e cases that spawn the
//! daemon with an empty agent list and no fake LLM.
//!
//! Single implementation of the config-tree block previously inlined in
//! `sigterm_tests` (SIGTERM/SIGINT cases) and `shutdown_checkpoint_tests`
//! (storage case) — STANDARDS §10: shared helper extracted into a common
//! module instead of being re-embedded per test case. Deliberately **not**
//! feature-gated: unlike the `config` submodule (feature `fake-llm`, full
//! fake-LLM config tree), this helper is reachable from cases built
//! without that feature.
//!
//! Contract tests live in the sibling `config_tree_tests` module
//! (STANDARDS §2: unit tests separated from the code, not inlined).

use std::path::Path;

/// Writes the minimal config tree a daemon needs to start up under
/// `<config_root>/config/`.
///
/// Creates `<config_root>/config/`, writes `agents.json` with an empty
/// agent list (`{"version":"1.0.0","agents":[]}`), then delegates to
/// [`closeclaw_common::test_helpers::write_mandatory_configs`] for the
/// mandatory config skeleton (channels/gateway/plugins/system/accounts
/// + models).
///
/// Only the config tree is written: the caller owns the temp directory
/// (`tempfile::TempDir`) that provides `config_root`, so the tree stays
/// alive for the duration of the test. IO errors propagate via `?`.
pub fn write_test_config_tree(config_root: &Path) -> std::io::Result<()> {
    let agents_dir = config_root.join("config");
    std::fs::create_dir_all(&agents_dir)?;
    std::fs::write(
        agents_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":[]}"#,
    )?;
    closeclaw_common::test_helpers::write_mandatory_configs(&agents_dir)
}
