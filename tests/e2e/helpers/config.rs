//! Shared config-tree scaffolding for fake-LLM e2e tests.
//!
//! Single implementation of the mandatory config skeleton previously
//! duplicated in `agent_profile_tests` (`write_mandatory_configs` +
//! `write_agent_layout`) and `gateway_restart_turn_tests`
//! (`write_config_tree`) — STANDARDS §10: shared helper extracted into a
//! common module instead of re-implemented per test file.

use std::path::Path;

/// `gateway.json` content (`gateway::GatewayConfig` snake_case form).
///
/// `load_gateway_config` parses the file as `GatewayConfig` (`name`
/// required, `max_message_size` defaults to 0 → every message rejected
/// as "消息过长"), so write explicit values (16384 = the startup default
/// in `Daemon::init_phase_2`).
const GATEWAY_CONFIG: &str = r#"{"name":"e2e","max_message_size":16384}"#;

/// Options for [`write_config_tree`].
pub struct ConfigTreeOpts {
    /// Address of the in-process fake LLM HTTP server `models.json`
    /// points its `baseUrl` at.
    fake_llm_addr: String,
    /// Also write `<root>/gateway.json` (restart-path quirk below).
    write_root_gateway: bool,
}

impl ConfigTreeOpts {
    /// Standard scaffold: config skeleton + master agent only.
    pub fn new(fake_llm_addr: impl std::fmt::Display) -> Self {
        Self {
            fake_llm_addr: fake_llm_addr.to_string(),
            write_root_gateway: false,
        }
    }

    /// Restart-path scaffold: additionally writes `<root>/gateway.json`.
    ///
    /// Restart-path quirk (documented, not fixed): `resolve_config_dir()`
    /// resolves the admin-socket parent = `<root>`, so the restart reads
    /// `<root>/gateway.json` — provide it there too.
    pub fn with_root_gateway(fake_llm_addr: impl std::fmt::Display) -> Self {
        Self {
            fake_llm_addr: fake_llm_addr.to_string(),
            write_root_gateway: true,
        }
    }
}

/// Write the full config tree shared by the fake-LLM e2e files: the
/// mandatory config skeleton (incl. `models.json` pointed at the fake LLM
/// and `models.json`'s `credentialPath` credentials) plus the default
/// `master` agent layout.
///
/// Layout (verified against `Daemon::init_phase_1_foundation` /
/// `ConfigManager::load` / `AgentDirectoryProvider`):
///
/// ```text
/// <root>/config/{models,channels,gateway,plugins,system,accounts}.json
/// <root>/config/agents.json
/// <root>/config/credentials/openai.json     (fake key; camelCase)
/// <root>/agents/master/config.json
/// ```
///
/// Notes:
/// - `models.json` `credentialPath` is validated with a CWD-relative
///   `Path::exists` check; the daemon child process gets its CWD set to
///   `<root>/config` via `Command::current_dir` in `helpers::spawn_daemon`,
///   so the relative credential path resolves under the config dir (the
///   test process itself never chdirs).
/// - `agents/<id>/config.json` `model` accepts `"provider/model-id"`
///   (ModelSpec string form). Tests needing a custom agent config
///   overwrite the master agent file after calling this function.
pub fn write_config_tree(root: &Path, opts: ConfigTreeOpts) {
    write_mandatory_configs(root, &opts);
    write_master_agent(root);
    if opts.write_root_gateway {
        // The restart path reads <root>/gateway.json (resolve_config_dir
        // returns the admin-socket parent, not the config subdir).
        std::fs::write(root.join("gateway.json"), GATEWAY_CONFIG)
            .expect("write root gateway.json for restart path");
    }
}

/// Write the mandatory config files (agents, models, gateway, channels,
/// plugins, system, accounts, credentials) under `<root>/config`.
fn write_mandatory_configs(root: &Path, opts: &ConfigTreeOpts) {
    let config_dir = root.join("config");
    std::fs::create_dir_all(config_dir.join("credentials")).expect("create config dirs");

    std::fs::write(
        config_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":["master"]}"#,
    )
    .expect("write agents.json");

    let models = serde_json::json!({
        "version": "1.0",
        "mode": "merge",
        "providers": {
            "openai": {
                "baseUrl": format!("http://{}/v1", opts.fake_llm_addr),
                "protocol": "openai",
                "credentialPath": "credentials/openai.json",
                "models": [{ "id": "gpt-4o-basic", "enabled": true }]
            }
        }
    });
    std::fs::write(
        config_dir.join("models.json"),
        serde_json::to_string(&models).expect("serialize models.json"),
    )
    .expect("write models.json");

    std::fs::write(config_dir.join("gateway.json"), GATEWAY_CONFIG).expect("write gateway.json");

    for name in [
        "channels.json",
        "plugins.json",
        "system.json",
        "accounts.json",
    ] {
        std::fs::write(config_dir.join(name), r#"{"version":"1.0"}"#)
            .expect("write mandatory config");
    }

    // Fake API key — camelCase per ApiKeyCredentials serde attrs.
    std::fs::write(
        config_dir.join("credentials").join("openai.json"),
        r#"{"provider":"openai","apiKey":"e2e-fake-key"}"#,
    )
    .expect("write credentials");
}

/// Write the default master agent layout (wildcard tool/skill
/// permissions, fake-LLM model reference) into the config tree.
fn write_master_agent(root: &Path) {
    std::fs::create_dir_all(root.join("agents").join("master")).expect("create agents dir");
    std::fs::write(
        root.join("agents").join("master").join("config.json"),
        r#"{"id":"master","name":"Master","model":"openai/gpt-4o-basic",
            "tools":["*"],"skills":["*"]}"#,
    )
    .expect("write master agent config");
}
