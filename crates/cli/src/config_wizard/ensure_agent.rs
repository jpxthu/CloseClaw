//! Initial agent bootstrap for the config wizard.

use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::{AgentConfig, AgentsConfig, AgentsConfigProvider};

use std::path::Path;

/// Create the initial `master` agent if it does not already exist.
///
/// Writes:
/// - `~/.closeclaw/agents/master/config.json`
/// - `~/.closeclaw/config/agents.json` (appends "master" if missing)
///
/// Idempotent: skips files that already exist.
pub(crate) fn ensure_master_agent(config_dir: &Path, agents_dir: &Path) -> anyhow::Result<()> {
    let master_id = crate::constants::DEFAULT_AGENT_ID;

    // ── Write master agent config.json if missing ──────────────────────────────
    let master_config_path = agents_dir.join(master_id).join("config.json");
    if !master_config_path.exists() {
        let dir = master_config_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("cannot determine agent directory"))?;
        std::fs::create_dir_all(dir)?;

        let config = AgentConfig {
            id: master_id.to_string(),
            bootstrap_mode: Some(BootstrapMode::Full),
            ..Default::default()
        };

        let json = serde_json::to_string_pretty(&config)
            .map_err(|e| anyhow::anyhow!("failed to serialize AgentConfig: {}", e))?;
        std::fs::write(&master_config_path, json)?;
    }

    // ── Update agents.json registration list ───────────────────────────────────
    let agents_json_path = config_dir.join("agents.json");
    if agents_json_path.exists() {
        let content = std::fs::read_to_string(&agents_json_path)?;
        let provider = AgentsConfigProvider::from_json_str(&content)?;
        let mut agents_config = provider.inner().clone();
        if !agents_config.agents.contains(&master_id.to_string()) {
            agents_config.agents.push(master_id.to_string());
            let json = serde_json::to_string_pretty(&agents_config)
                .map_err(|e| anyhow::anyhow!("failed to serialize agents.json: {}", e))?;
            std::fs::write(&agents_json_path, json)?;
        }
    } else {
        let agents_config = AgentsConfig {
            agents: vec![master_id.to_string()],
        };
        let json = serde_json::to_string_pretty(&agents_config)
            .map_err(|e| anyhow::anyhow!("failed to serialize agents.json: {}", e))?;
        std::fs::write(&agents_json_path, json)?;
    }

    Ok(())
}
