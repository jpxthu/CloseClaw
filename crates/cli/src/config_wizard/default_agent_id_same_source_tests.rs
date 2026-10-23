//! Same-source invariant tests for the default agent id.
//!
//! Invariant under test: the `chat --agent-id` default value equals the id of
//! the initial agent created by the config wizard (docs/design/agent/agent-config.md).

use super::*;
use clap::Parser;
use tempfile::TempDir;

/// Wrapper to resolve the `chat` subcommand's default `--agent-id` via clap.
#[derive(clap::Parser, Debug)]
struct ChatDefaultCli {
    #[command(flatten)]
    args: crate::args::ChatArgs,
}

/// 同源不变量：chat `--agent-id` 缺省值 = 配置向导创建的初始 Agent id。
///
/// 在 TempDir 上执行 `ensure_master_agent`，断言 chat 子命令解析出的缺省
/// `agent_id` 与 crate 共享常量 `DEFAULT_AGENT_ID`、写入的
/// `agents/<id>/config.json` 的 id、agents.json 清单同源一致。
#[test]
fn test_chat_default_agent_id_matches_wizard_initial_agent() {
    // chat 子命令缺省 agent_id（不带 --agent-id 解析）
    let cli = ChatDefaultCli::try_parse_from(["test"]).unwrap();
    let chat_default_id = cli.args.agent_id;

    // 缺省值必须来自 crate 共享常量
    assert_eq!(
        chat_default_id,
        crate::constants::DEFAULT_AGENT_ID,
        "chat --agent-id default must equal crate shared DEFAULT_AGENT_ID"
    );

    // 配置向导在 TempDir 上创建初始 Agent（不触碰真实 ~/.closeclaw）。
    // config_dir 直接复用 TempDir 根（已存在）；agents 子目录由
    // ensure_master_agent 内部自建，无需手动创建。
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path();
    let agents_dir = tmp.path().join("agents");
    ensure_master_agent(config_dir, &agents_dir).unwrap();

    // agents.json 清单包含 chat 缺省 agent id
    let agents_json_path = config_dir.join("agents.json");
    let content = std::fs::read_to_string(&agents_json_path).unwrap();
    let agents_config: AgentsConfig = serde_json::from_str(&content).unwrap();
    assert!(
        agents_config.agents.iter().any(|a| a == &chat_default_id),
        "agents.json manifest should contain chat default agent id {chat_default_id:?}, got: {:?}",
        agents_config.agents
    );

    // agents/<id>/config.json 的 id 与 chat 缺省 agent id 一致
    let config_path = agents_dir.join(&chat_default_id).join("config.json");
    let content =
        std::fs::read_to_string(&config_path).expect("wizard-created config.json should exist");
    let agent_config: AgentConfig = serde_json::from_str(&content).unwrap();
    assert_eq!(
        agent_config.id, chat_default_id,
        "wizard-created initial agent id must equal chat --agent-id default"
    );
}
