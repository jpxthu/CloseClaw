//! Skill discovery skill - allows agents to search and install
//! skills from ClawHub
use crate::disk::types::SkillEffort;
use crate::registry::{Skill, SkillError, SkillListingMeta, SkillManifest};
use async_trait::async_trait;
use serde_json::json;
use tokio::task::spawn_blocking;

#[derive(Default)]
pub struct SkillDiscoverySkill;

impl SkillDiscoverySkill {
    pub fn new() -> Self {
        Self
    }

    /// Build the default capability description returned when no
    /// action is specified.
    fn capabilities_description() -> String {
        json!({
            "skill": "skill_discovery",
            "description": "Search, install, and manage skills from ClawHub marketplace",
            "supported_actions": ["find", "install", "list", "update"],
            "usage": {
                "find": {"query": "<search_query>"},
                "install": {"name": "<skill_name>", "version": "<optional_version>"},
                "list": {},
                "update": {"name": "<optional_skill_name>"}
            }
        })
        .to_string()
    }

    /// Run a clawhub CLI command and return its stdout.
    async fn run_clawhub(args: &[&str]) -> Result<String, SkillError> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        spawn_blocking(move || {
            let output = std::process::Command::new("clawhub")
                .args(&args)
                .output()
                .map_err(|e| SkillError::ExecutionFailed(format!("failed to run clawhub: {e}")))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(SkillError::ExecutionFailed(format!(
                    "clawhub {} failed: {}",
                    args.join(" "),
                    stderr.trim()
                )));
            }
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        })
        .await
        .map_err(|e| SkillError::ExecutionFailed(format!("task join error: {e}")))?
    }

    /// Execute `clawhub search <query>`.
    async fn execute_find(query: &str) -> Result<String, SkillError> {
        let stdout = Self::run_clawhub(&["search", query]).await?;
        Ok(json!({
            "action": "find",
            "query": query,
            "results": stdout.trim()
        })
        .to_string())
    }

    /// Execute `clawhub install <name>` with optional version.
    async fn execute_install(name: &str, version: Option<&str>) -> Result<String, SkillError> {
        let mut args = vec!["install", name];
        let ver_str;
        if let Some(v) = version {
            ver_str = v.to_string();
            args.push("--version");
            args.push(&ver_str);
        }
        let stdout = Self::run_clawhub(&args).await?;
        Ok(json!({
            "action": "install",
            "name": name,
            "version": version,
            "output": stdout.trim()
        })
        .to_string())
    }

    /// Execute `clawhub list`.
    async fn execute_list() -> Result<String, SkillError> {
        let stdout = Self::run_clawhub(&["list"]).await?;
        Ok(json!({
            "action": "list",
            "installed": stdout.trim()
        })
        .to_string())
    }

    /// Execute `clawhub update [name]`.
    async fn execute_update(name: Option<&str>) -> Result<String, SkillError> {
        let stdout = if let Some(n) = name {
            Self::run_clawhub(&["update", n]).await?
        } else {
            Self::run_clawhub(&["update", "--all"]).await?
        };
        Ok(json!({
            "action": "update",
            "name": name,
            "output": stdout.trim()
        })
        .to_string())
    }
}

#[async_trait]
impl Skill for SkillDiscoverySkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "skill_discovery".to_string(),
            version: "1.0.0".to_string(),
            description: "Search, install, and manage skills from \
                ClawHub marketplace. Use find to search, install to \
                add, list to see installed, update to upgrade."
                .to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec!["clawhub".to_string()],
        }
    }

    fn body(&self) -> &str {
        r#"# Skill Discovery Skill

Use the `exec` tool to run `clawhub` CLI commands for skill management:

- **Search**: `exec` with `clawhub search <query>`
- **Install**: `exec` with `clawhub install <skill-name>` (optionally `--version <version>`)
- **List installed**: `exec` with `clawhub list`
- **Update**: `exec` with `clawhub update [skill-name]` (or `--all` for all skills)

Always confirm before installing or updating skills."#
    }

    async fn execute(&self, args: Option<serde_json::Value>) -> Result<String, SkillError> {
        let args = match args {
            Some(a) => a,
            None => return Ok(Self::capabilities_description()),
        };

        let action = args.get("action").and_then(|v| v.as_str());

        match action {
            None => Ok(Self::capabilities_description()),
            Some("find") => {
                let query = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'query' for find action".into())
                })?;
                Self::execute_find(query).await
            }
            Some("install") => {
                let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'name' for install action".into())
                })?;
                let version = args.get("version").and_then(|v| v.as_str());
                Self::execute_install(name, version).await
            }
            Some("list") => Self::execute_list().await,
            Some("update") => {
                let name = args.get("name").and_then(|v| v.as_str());
                Self::execute_update(name).await
            }
            Some(other) => Err(SkillError::InvalidArgs(format!(
                "unknown action '{other}', supported: find, install, list, update"
            ))),
        }
    }

    fn listing_meta(&self) -> SkillListingMeta {
        SkillListingMeta {
            when_to_use: "Use when the agent needs to search, install, \
                or manage skills from ClawHub marketplace"
                .to_string(),
            user_invocable: true,
            paths: vec![],
            effort: SkillEffort::Small,
        }
    }
}
