//! Shared config-tree scaffolding for fake-LLM e2e tests.
//!
//! Single implementation of the mandatory config skeleton previously
//! duplicated in `agent_profile_tests` (`write_mandatory_configs` +
//! `write_agent_layout`) and `gateway_restart_turn_tests`
//! (`write_config_tree`) — STANDARDS §10: shared helper extracted into a
//! common module instead of re-implemented per test file.
//!
//! This module builds the **full fake-LLM** config tree; for the minimal,
//! non-feature-gated tree (empty `agents.json` + mandatory configs only)
//! see [`super::config_tree::write_test_config_tree`] to avoid mixing up
//! the two similarly named helpers.

use std::path::Path;

/// `gateway.json` content (`gateway::GatewayConfig` snake_case form).
///
/// `load_gateway_config` parses the file as `GatewayConfig` (`name`
/// required, `max_message_size` defaults to 0 → every message rejected
/// as "消息过长"), so write explicit values (16384 = the startup default
/// in `Daemon::init_phase_2`).
const GATEWAY_CONFIG: &str = r#"{"name":"e2e","max_message_size":16384}"#;

/// Default `models.json` model id (single-entry fallback chain).
const DEFAULT_MODEL_ID: &str = "gpt-4o-basic";

/// Options for [`write_config_tree`].
pub struct ConfigTreeOpts {
    /// Address of the in-process fake LLM HTTP server `models.json`
    /// points its `baseUrl` at.
    fake_llm_addr: String,
    /// Also write `<root>/gateway.json` (restart-path quirk below).
    write_root_gateway: bool,
    /// Enabled `models.json` model ids, in fallback-chain order.
    model_ids: Vec<String>,
}

impl ConfigTreeOpts {
    /// Standard scaffold: config skeleton + master agent only.
    pub fn new(fake_llm_addr: impl std::fmt::Display) -> Self {
        Self {
            fake_llm_addr: fake_llm_addr.to_string(),
            write_root_gateway: false,
            model_ids: vec![DEFAULT_MODEL_ID.to_string()],
        }
    }

    /// Restart-path scaffold: additionally writes `<root>/gateway.json`.
    ///
    /// Restart-path quirk (documented, not fixed): `resolve_config_dir()`
    /// resolves the admin-socket parent = `<root>`, so the restart reads
    /// `<root>/gateway.json` — provide it there too.
    pub fn with_root_gateway(fake_llm_addr: impl std::fmt::Display) -> Self {
        let mut opts = Self::new(fake_llm_addr);
        opts.write_root_gateway = true;
        opts
    }

    /// Declare this test's enabled `models.json` models (chain order).
    ///
    /// Replaces the default single `gpt-4o-basic` entry. The daemon
    /// builds one fallback-chain entry per enabled model in order, and
    /// `UnifiedFallbackClient` overwrites `request.model` with each
    /// entry's id starting at index 0 — so the FIRST id here is the
    /// model observed on the wire (and matched by fake_llm scenarios).
    /// The master agent config's `model` follows that first id
    /// (`openai/<id>`, see `write_master_agent`), so the scaffold stays
    /// aligned without a separate agent-config overwrite.
    pub fn with_models(mut self, model_ids: &[&str]) -> Self {
        self.model_ids = model_ids.iter().map(|id| id.to_string()).collect();
        self
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
    write_master_agent(root, &opts);
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

    let models = models_json(opts);
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

/// Build the `models.json` value: one enabled entry per declared model
/// id, in declaration order (chain order).
fn models_json(opts: &ConfigTreeOpts) -> serde_json::Value {
    let models: Vec<serde_json::Value> = opts
        .model_ids
        .iter()
        .map(|id| serde_json::json!({ "id": id, "enabled": true }))
        .collect();
    serde_json::json!({
        "version": "1.0",
        "mode": "merge",
        "providers": {
            "openai": {
                "baseUrl": format!("http://{}/v1", opts.fake_llm_addr),
                "protocol": "openai",
                "credentialPath": "credentials/openai.json",
                "models": models
            }
        }
    })
}

/// Write a custom agent `config.json` into the config tree.
///
/// Overwrites the master agent config created by the shared
/// `write_config_tree` scaffold to set a specific `model` and/or
/// `workspace` field. Uses `serde_json::json!` as the single
/// authoritative agent-config construction point.
pub fn write_agent_config(config_root: &Path, model: &str, workspace: Option<&str>) {
    let agent_dir = config_root.join("agents").join("master");
    std::fs::create_dir_all(&agent_dir).expect("create agent dir");
    let mut config = serde_json::json!({
        "id": "master",
        "name": "Master",
        "model": model,
        "tools": ["*"],
        "skills": ["*"]
    });
    if let Some(ws) = workspace {
        config["workspace"] = serde_json::Value::String(ws.to_string());
    }
    std::fs::write(
        agent_dir.join("config.json"),
        serde_json::to_string(&config).expect("serialize agent config"),
    )
    .expect("write agent config");
}

/// Write agent config with explicit tools whitelist and disallowed-tools
/// blacklist.
///
/// Like [`write_agent_config`] but allows fine-grained control over the
/// `tools` whitelist and the `disallowedTools` blacklist. On-disk field
/// names follow `docs/design/agent/agent-config.md` (camelCase — the
/// `AgentConfig` parser renames `disallowed_tools` → `disallowedTools`;
/// a snake_case key is silently ignored as an unknown field).
///
/// `tools` / `disallowed` entries must spell the registry `Tool::name()`
/// exactly (`Read`, `Bash`, …): the prompt-side descriptor filter and
/// the execution-time agent-tools gate both compare case-sensitively.
pub fn write_agent_config_with_tools(
    config_root: &Path,
    model: &str,
    tools: &[&str],
    disallowed: &[&str],
) {
    let agent_dir = config_root.join("agents").join("master");
    std::fs::create_dir_all(&agent_dir).expect("create agent dir");
    let tools: Vec<serde_json::Value> = tools.iter().map(|t| serde_json::json!(t)).collect();
    let disallowed: Vec<serde_json::Value> =
        disallowed.iter().map(|t| serde_json::json!(t)).collect();
    std::fs::write(
        agent_dir.join("config.json"),
        serde_json::json!({
            "id": "master",
            "name": "Master",
            "model": model,
            "tools": tools,
            "disallowedTools": disallowed,
            "skills": ["*"]
        })
        .to_string(),
    )
    .expect("write agent config with tools");
}

/// Write the default master agent layout (wildcard tool/skill
/// permissions, model reference derived from the first declared
/// `models.json` id) into the config tree.
///
/// Delegates to [`write_agent_config`] for the actual write.
fn write_master_agent(root: &Path, opts: &ConfigTreeOpts) {
    let model = format!(
        "openai/{}",
        opts.model_ids
            .first()
            .expect("model_ids must declare at least one id")
    );
    write_agent_config(root, &model, None);
}

/// Write `<root>/agents/<agent_id>/permissions.json` — a permission
/// engine `RuleSet` (`{"rules": [...]}`; schema:
/// `crates/permission/src/engine/engine_types.rs`).
///
/// This is the exact path the daemon-side engine lazily loads
/// (`{data_root}/agents/{agent_id}/permissions.json`,
/// `engine_agent_rules.rs`; `data_root` = the config root passed as
/// `--config-dir`, i.e. `root` here).
///
/// Note: only this file's `rules` array is merged with the global rule
/// set — the file's own `defaults`/`user_defaults` are dropped by the
/// loader, and the global `tool_call` default stays Deny — so grants
/// must be explicit rules (see `docs/design/permission/README.md`
/// §规则加载策略 / §交集模型). The concrete rule JSON is test-case
/// data and stays with the caller, not in this shared scaffold.
pub fn write_agent_permissions(root: &Path, agent_id: &str, rule_set_json: &str) {
    let agent_dir = root.join("agents").join(agent_id);
    std::fs::create_dir_all(&agent_dir).expect("create agent dir for permissions");
    std::fs::write(agent_dir.join("permissions.json"), rule_set_json)
        .expect("write agent permissions.json");
}
