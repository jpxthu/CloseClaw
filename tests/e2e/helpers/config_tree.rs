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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::write_test_config_tree;

    /// The exact `agents.json` payload the helper must write (byte-for-byte).
    const EXPECTED_AGENTS_JSON: &str = r#"{"version":"1.0.0","agents":[]}"#;

    /// Files produced by `closeclaw_common::test_helpers::write_mandatory_configs`:
    /// the 5 mandatory configs plus the placeholder models.json.
    const MANDATORY_FILES: [&str; 6] = [
        "accounts.json",
        "channels.json",
        "gateway.json",
        "models.json",
        "plugins.json",
        "system.json",
    ];

    /// Asserts the helper's output contract under `config_root`:
    /// `<root>/config/` exists, `agents.json` holds the empty agent list,
    /// and every mandatory config file is present.
    fn assert_config_tree_contract(config_root: &Path) {
        let config_dir = config_root.join("config");
        assert!(
            config_dir.is_dir(),
            "expected config dir {} to exist as a directory",
            config_dir.display()
        );

        let agents = std::fs::read_to_string(config_dir.join("agents.json"))
            .expect("agents.json should be readable");
        assert_eq!(
            agents, EXPECTED_AGENTS_JSON,
            "agents.json must hold exactly the empty agent list"
        );

        for name in MANDATORY_FILES {
            let path = config_dir.join(name);
            assert!(
                path.is_file(),
                "expected mandatory config {} to exist under {}",
                name,
                config_dir.display()
            );
        }
    }

    /// Normal path: a fresh temp root gets the full contract tree —
    /// `config/` dir, empty `agents.json`, and all mandatory configs.
    #[test]
    fn test_write_test_config_tree_on_fresh_root() {
        let root = tempfile::TempDir::new().expect("create temp dir under /tmp");

        write_test_config_tree(root.path()).expect("write config tree into a fresh root");

        assert_config_tree_contract(root.path());
    }

    /// Boundary: a pre-existing `<root>/config/` (with stale contents) must not
    /// panic and must be overwritten; a second call stays idempotent.
    #[test]
    fn test_write_test_config_tree_overwrites_existing_config_dir() {
        let root = tempfile::TempDir::new().expect("create temp dir under /tmp");
        let config_dir = root.path().join("config");
        std::fs::create_dir_all(&config_dir).expect("pre-create config dir");
        std::fs::write(config_dir.join("agents.json"), "stale agents payload")
            .expect("pre-seed stale agents.json");

        write_test_config_tree(root.path()).expect("existing config dir must not panic");
        assert_config_tree_contract(root.path());

        write_test_config_tree(root.path()).expect("second call must stay idempotent");
        assert_config_tree_contract(root.path());
    }

    /// Error path: a regular file shadowing `config_root` surfaces `Err`
    /// (propagated through `?`) instead of panicking.
    #[test]
    fn test_write_test_config_tree_errors_when_config_root_is_a_file() {
        let root = tempfile::TempDir::new().expect("create temp dir under /tmp");
        let blocked_root = root.path().join("blocked_root");
        std::fs::write(&blocked_root, "regular file, not a directory")
            .expect("create blocking regular file");

        let result = write_test_config_tree(&blocked_root);
        assert!(
            result.is_err(),
            "config_root shadowed by a regular file should return Err, got Ok"
        );
    }
}
