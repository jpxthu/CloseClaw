//! Permission Engine - Core security component
//!
//! Runs as a separate OS process, evaluates access rules for agents.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;

/// RuleSet parsed from permissions.json
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleSet {
    pub version: String,
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub defaults: Defaults,
}

/// Default permissions for each action type
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Defaults {
    #[serde(default = "default_deny")]
    pub file: Effect,
    #[serde(default = "default_deny")]
    pub command: Effect,
    #[serde(default = "default_deny")]
    pub network: Effect,
    #[serde(default = "default_deny")]
    pub inter_agent: Effect,
    #[serde(default = "default_deny")]
    pub config: Effect,
}

fn default_deny() -> Effect {
    Effect::Deny
}

/// A single permission rule
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub name: String,
    pub subject: Subject,
    pub effect: Effect,
    pub actions: Vec<Action>,
}

/// Subject that a rule applies to
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Subject {
    pub agent: String,
    #[serde(default, alias = "match")]
    pub match_type: MatchType,
}

impl Subject {
    pub fn matches(&self, agent_id: &str) -> bool {
        match self.match_type {
            MatchType::Exact => self.agent == agent_id,
            MatchType::Glob => glob_match(&self.agent, agent_id),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    #[default]
    Exact,
    Glob,
}

/// An action that a rule permits or denies
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    File {
        operation: String,
        paths: Vec<String>,
    },
    Command {
        command: String,
        #[serde(default)]
        args: CommandArgs,
    },
    Network {
        #[serde(default)]
        hosts: Vec<String>,
        #[serde(default)]
        ports: Vec<u16>,
    },
    ToolCall {
        skill: String,
        #[serde(default)]
        methods: Vec<String>,
    },
    InterAgent {
        #[serde(default)]
        agents: Vec<String>,
    },
    ConfigWrite {
        #[serde(default)]
        files: Vec<String>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(untagged)]
pub enum CommandArgs {
    #[default]
    Any,
    Allowed { allowed: Vec<String> },
    Blocked { blocked: Vec<String> },
}

/// Permission effect
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    #[default]
    Deny,
    Allow,
}

/// Permission request from an agent
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionRequest {
    FileOp {
        agent: String,
        path: String,
        op: String,
    },
    CommandExec {
        agent: String,
        cmd: String,
        args: Vec<String>,
    },
    NetOp {
        agent: String,
        host: String,
        port: u16,
    },
    ToolCall {
        agent: String,
        skill: String,
        method: String,
    },
    InterAgentMsg {
        from: String,
        to: String,
    },
    ConfigWrite {
        agent: String,
        config_file: String,
    },
}

/// Permission response from the engine
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PermissionResponse {
    Allowed { token: String },
    Denied { reason: String, rule: String },
}

/// Permission Engine - evaluates access requests against rules
pub struct PermissionEngine {
    rules: RwLock<RuleSet>,
    /// O(1) lookup index: agent_id -> list of rule indices
    agent_rule_index: RwLock<HashMap<String, Vec<usize>>>,
}

impl PermissionEngine {
    /// Create a new PermissionEngine from a RuleSet
    pub fn new(rules: RuleSet) -> Self {
        let mut agent_rule_index: HashMap<String, Vec<usize>> = HashMap::new();
        
        for (idx, rule) in rules.rules.iter().enumerate() {
            let agent_id = &rule.subject.agent;
            agent_rule_index
                .entry(agent_id.clone())
                .or_default()
                .push(idx);
        }
        
        Self {
            rules: RwLock::new(rules),
            agent_rule_index: RwLock::new(agent_rule_index),
        }
    }

    /// Evaluate a permission request
    pub async fn evaluate(&self, request: PermissionRequest) -> PermissionResponse {
        let rules = self.rules.read().await;
        let agent_id = request.agent_id();
        
        // O(1) lookup via index, then fall back to glob matching
        let rule_indices = match self.agent_rule_index.read().await.get(agent_id) {
            Some(indices) => indices.clone(),
            None => {
                // Try glob matching against all rule subjects
                let mut matched_indices = Vec::new();
                for (idx, rule) in rules.rules.iter().enumerate() {
                    if rule.subject.matches(agent_id) {
                        matched_indices.push(idx);
                    }
                }
                if matched_indices.is_empty() {
                    // Unknown agent - use defaults
                    return self.default_deny(&request, &rules.defaults, "unknown agent");
                }
                matched_indices
            }
        };
        
        // Evaluate matching rules
        let mut explicit_effect: Option<Effect> = None;
        let mut last_rule = String::new();
        
        for &idx in &rule_indices {
            let rule = &rules.rules[idx];
            if self.rule_matches_request(rule, &request) {
                last_rule = rule.name.clone();
                explicit_effect = Some(rule.effect);
                
                // Deny takes precedence (AWS IAM style)
                if rule.effect == Effect::Deny {
                    return PermissionResponse::Denied {
                        reason: format!("action denied by rule '{}'", rule.name),
                        rule: rule.name.clone(),
                    };
                }
            }
        }
        
        match explicit_effect {
            Some(Effect::Allow) => PermissionResponse::Allowed {
                token: generate_token(),
            },
            None => self.default_deny(&request, &rules.defaults, "no matching rule"),
            Some(Effect::Deny) => PermissionResponse::Denied {
                reason: format!("action denied by rule '{}'", last_rule),
                rule: last_rule,
            },
        }
    }
    
    /// Get default action when no rule matches
    fn default_deny(&self, request: &PermissionRequest, defaults: &Defaults, reason: &str) -> PermissionResponse {
        let effect = match request {
            PermissionRequest::FileOp { .. } => defaults.file,
            PermissionRequest::CommandExec { .. } => defaults.command,
            PermissionRequest::NetOp { .. } => defaults.network,
            PermissionRequest::InterAgentMsg { .. } => defaults.inter_agent,
            PermissionRequest::ConfigWrite { .. } => defaults.config,
            PermissionRequest::ToolCall { .. } => defaults.file, // Tools are file-related
        };
        
        match effect {
            Effect::Allow => PermissionResponse::Allowed { token: generate_token() },
            Effect::Deny => PermissionResponse::Denied {
                reason: reason.to_string(),
                rule: "default".to_string(),
            },
        }
    }
    
    /// Check if a rule's actions match the request
    fn rule_matches_request(&self, rule: &Rule, request: &PermissionRequest) -> bool {
        for action in &rule.actions {
            match (action, request) {
                (Action::File { operation, paths }, PermissionRequest::FileOp { path, op, .. }) => {
                    if operation == op && paths.iter().any(|p| glob_match(p, path)) {
                        return true;
                    }
                }
                (Action::Command { command, args }, PermissionRequest::CommandExec { cmd, args: req_args, .. }) => {
                    if command == cmd && self.args_match(args, req_args) {
                        return true;
                    }
                }
                (Action::Network { hosts, ports }, PermissionRequest::NetOp { host, port, .. }) => {
                    if (hosts.is_empty() || hosts.iter().any(|h| glob_match(h, host)))
                        && (ports.is_empty() || ports.contains(port))
                    {
                        return true;
                    }
                }
                (Action::ToolCall { skill, methods }, PermissionRequest::ToolCall { skill: s, method, .. }) => {
                    if skill == s && (methods.is_empty() || methods.contains(method)) {
                        return true;
                    }
                }
                (Action::InterAgent { agents }, PermissionRequest::InterAgentMsg { to, .. }) => {
                    if agents.is_empty() || agents.iter().any(|a| glob_match(a, to)) {
                        return true;
                    }
                }
                (Action::ConfigWrite { files }, PermissionRequest::ConfigWrite { config_file, .. }) => {
                    if files.is_empty() || files.iter().any(|f| glob_match(f, config_file)) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    
    /// Check if command arguments match
    /// For Allowed: returns true if ALL request args are in the allowed list
    /// For Blocked: returns true if ANY request arg is in the blocked list (i.e., should be blocked)
    pub fn args_match(&self, rule_args: &CommandArgs, request_args: &[String]) -> bool {
        match rule_args {
            CommandArgs::Any => true,
            CommandArgs::Allowed { allowed } => {
                // All request args must be in the allowed list
                request_args.iter().all(|arg| allowed.iter().any(|a| glob_match(a, arg)))
            }
            CommandArgs::Blocked { blocked } => {
                // True if ANY request arg is in the blocked list
                request_args.iter().any(|arg| blocked.iter().any(|b| glob_match(b, arg)))
            }
        }
    }
}

impl PermissionRequest {
    /// Extract agent ID from request
    pub fn agent_id(&self) -> &str {
        match self {
            PermissionRequest::FileOp { agent, .. } => agent,
            PermissionRequest::CommandExec { agent, .. } => agent,
            PermissionRequest::NetOp { agent, .. } => agent,
            PermissionRequest::ToolCall { agent, .. } => agent,
            PermissionRequest::InterAgentMsg { from, .. } => from,
            PermissionRequest::ConfigWrite { agent, .. } => agent,
        }
    }
}

/// Simple glob matching (supports * and **)
pub fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "**" || pattern == "*" {
        return true;
    }
    
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    
    glob_match_vec(&pattern_chars, &text_chars, 0, 0)
}

fn glob_match_vec(pat: &[char], text: &[char], pi: usize, ti: usize) -> bool {
    if pi == pat.len() && ti == text.len() {
        return true;
    }
    
    if pi < pat.len() && pat[pi] == '*' {
        // ** matches anything
        if pi + 1 < pat.len() && pat[pi + 1] == '*' {
            // ** followed by / or end
            if pi + 2 < pat.len() && (pat[pi + 2] == '/' || pat[pi + 2] == '\\') {
                // / ** / or \ ** \ - skip the directory
                if pi + 3 < pat.len() {
                    return glob_match_vec(pat, text, pi + 3, ti)
                        || (ti < text.len() && glob_match_vec(pat, text, pi, ti + 1));
                }
                return ti >= text.len() || text[ti] == '/';
            }
            // Simple ** - match anything
            return ti >= text.len()
                || glob_match_vec(pat, text, pi + 2, ti)
                || glob_match_vec(pat, text, pi, ti + 1);
        }
        // * matches anything except /
        if ti >= text.len() {
            // No more text - * matches empty, continue to see if rest of pattern can finish
            return glob_match_vec(pat, text, pi + 1, ti);
        }
        return text[ti] != '/'
            && (glob_match_vec(pat, text, pi + 1, ti) || glob_match_vec(pat, text, pi, ti + 1));
    }
    
    if pi < pat.len() && ti < text.len() && (pat[pi] == '?' || pat[pi] == text[ti]) {
        return glob_match_vec(pat, text, pi + 1, ti + 1);
    }
    
    false
}

/// Generate a short-lived permission token
fn generate_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    format!("perm_{}_{:016x}", duration.as_secs(), rand_u64())
}

fn rand_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    RandomState::new().build_hasher().finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_glob_exact() {
        assert!(glob_match("dev-agent-01", "dev-agent-01"));
        assert!(!glob_match("dev-agent-01", "dev-agent-02"));
    }
    
    #[test]
    fn test_glob_star() {
        assert!(glob_match("readonly-*", "readonly-agent-1"));
        assert!(glob_match("readonly-*", "readonly-agent-42"));
        assert!(!glob_match("readonly-*", "readonly"));
    }
    
    #[test]
    fn test_glob_double_star() {
        assert!(glob_match("/home/admin/code/**", "/home/admin/code/closeclaw/src/main.rs"));
        assert!(glob_match("/home/admin/code/**", "/home/admin/code/closeclaw/src/permission/engine.rs"));
        assert!(!glob_match("/home/admin/code/**", "/home/admin/other/path"));
    }
    
    #[test]
    fn test_glob_question() {
        assert!(glob_match("file_?.txt", "file_a.txt"));
        assert!(glob_match("file_?.txt", "file_1.txt"));
        assert!(!glob_match("file_?.txt", "file_12.txt"));
    }

    // -------------------------------------------------------------------------
    // Tests from tests/engine_test.rs (rule parsing + action types)
    // -------------------------------------------------------------------------

    fn test_rules_json() -> &'static str {
        r#"{
  "version": "1.0",
  "rules": [
    {
      "name": "dev-agent-file-read",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "file",
          "operation": "read",
          "paths": ["/home/admin/code/**"]
        }
      ]
    },
    {
      "name": "dev-agent-file-write",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "file",
          "operation": "write",
          "paths": ["/home/admin/code/closeclaw/src/**"]
        }
      ]
    },
    {
      "name": "dev-agent-git",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "allowed": ["status", "log", "diff", "add", "commit", "push", "pull"] }
        }
      ]
    },
    {
      "name": "dev-agent-forbidden-git-reset",
      "subject": { "agent": "dev-agent-01" },
      "effect": "deny",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "blocked": ["reset", "rebase", "push", "--force"] }
        }
      ]
    },
    {
      "name": "readonly-agent",
      "subject": { "agent": "readonly-*", "match": "glob" },
      "effect": "allow",
      "actions": [
        { "type": "file", "operation": "read", "paths": ["**"] }
      ]
    }
  ],
  "defaults": {
    "file": "deny",
    "command": "deny",
    "network": "deny",
    "inter_agent": "deny",
    "config": "deny"
  }
}"#
    }

    #[tokio::test]
    async fn test_rule_parsing() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        assert_eq!(rules.version, "1.0");
        assert_eq!(rules.rules.len(), 5);
        assert_eq!(rules.defaults.file, Effect::Deny);
    }

    #[tokio::test]
    async fn test_file_read_allowed() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/home/admin/code/closeclaw/src/main.rs".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_file_read_denied_no_match() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/etc/passwd".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_file_write_allowed() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::FileOp {
            agent: "dev-agent-01".to_string(),
            path: "/home/admin/code/closeclaw/src/main.rs".to_string(),
            op: "write".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_command_allowed() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::CommandExec {
            agent: "dev-agent-01".to_string(),
            cmd: "git".to_string(),
            args: vec!["status".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_command_denied_blocked() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::CommandExec {
            agent: "dev-agent-01".to_string(),
            cmd: "git".to_string(),
            args: vec!["reset".to_string(), "--hard".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_glob_matching() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::FileOp {
            agent: "readonly-agent-42".to_string(),
            path: "/any/path/in/the/system.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_default_deny() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::NetOp {
            agent: "dev-agent-01".to_string(),
            host: "example.com".to_string(),
            port: 443,
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_network_action_type() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::NetOp {
            agent: "dev-agent-01".to_string(),
            host: "api.github.com".to_string(),
            port: 443,
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_tool_call_action_type() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::ToolCall {
            agent: "dev-agent-01".to_string(),
            skill: "file_ops".to_string(),
            method: "read_file".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_inter_agent_action_type() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::InterAgentMsg {
            from: "agent-a".to_string(),
            to: "agent-b".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_config_write_action_type() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::ConfigWrite {
            agent: "dev-agent-01".to_string(),
            config_file: "agents.json".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_rule_subject_matching_exact() {
        let rule = Rule::parse_subject("dev-agent-01");
        assert!(rule.matches("dev-agent-01"));
        assert!(!rule.matches("dev-agent-02"));
    }

    #[tokio::test]
    async fn test_rule_subject_matching_glob() {
        let rule = Rule::parse_subject_with_match("readonly-*", "glob");
        assert!(rule.matches("readonly-agent-1"));
        assert!(rule.matches("readonly-agent-42"));
        assert!(!rule.matches("readonly"));
    }

    #[tokio::test]
    async fn test_o1_lookup_performance() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let request = PermissionRequest::FileOp {
                agent: "dev-agent-01".to_string(),
                path: "/home/admin/code/closeclaw/src/main.rs".to_string(),
                op: "read".to_string(),
            };
            let _ = engine.evaluate(request);
        }
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 100, "O(1) lookup should be fast, took {:?}", elapsed);
    }

    #[tokio::test]
    async fn test_unknown_agent_defaults_to_deny() {
        let json = test_rules_json();
        let rules: RuleSet = serde_json::from_str(json).expect("Failed to parse rules");
        let engine = PermissionEngine::new(rules);
        let request = PermissionRequest::FileOp {
            agent: "unknown-agent".to_string(),
            path: "/home/admin/code/**".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    // -------------------------------------------------------------------------
    // Tests from tests/smoke_test.rs (PermissionEngine portion)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_permission_engine_parse() {
        let json = r#"{
            "version": "1.0",
            "rules": [],
            "defaults": { "effect": "deny" }
        }"#;
        let _rules: RuleSet = serde_json::from_str(json).unwrap();
    }

    // -------------------------------------------------------------------------
    // Comprehensive glob_match corner case tests (from comprehensive_tests.rs)
    // -------------------------------------------------------------------------

    #[test]
    fn test_glob_match_exact_match() {
        assert!(glob_match("dev-agent-01", "dev-agent-01"));
        assert!(!glob_match("dev-agent-01", "dev-agent-02"));
    }

    #[test]
    fn test_glob_match_double_star_matches_anything() {
        assert!(glob_match("**", "anything"));
        assert!(glob_match("**", "/home/admin/secret"));
        assert!(glob_match("**", "simple"));
    }

    #[test]
    fn test_glob_match_single_star_matches_anything_except_slash() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", "simple"));
        assert!(glob_match("file_*.txt", "file_read.txt"));
        assert!(!glob_match("file_*.txt", "file_read/write.txt"));
    }

    #[test]
    fn test_glob_match_question_matches_single_char() {
        assert!(glob_match("file_?.txt", "file_a.txt"));
        assert!(glob_match("file_?.txt", "file_1.txt"));
        assert!(glob_match("file_?.txt", "file_z.txt"));
        assert!(!glob_match("file_?.txt", "file_ab.txt"));
        assert!(!glob_match("file_?.txt", "file_.txt"));
    }

    #[test]
    fn test_glob_match_empty_pattern() {
        assert!(!glob_match("", "anything"));
        assert!(glob_match("", ""));
    }

    #[test]
    fn test_glob_match_path_with_directory_separators() {
        assert!(glob_match("/home/admin/**", "/home/admin/code/closeclaw/src/main.rs"));
        assert!(glob_match("/home/admin/**", "/home/admin/code"));
    }

    #[test]
    fn test_glob_match_directory_star_does_not_match_slash() {
        assert!(!glob_match("*/file.txt", "dir/file.txt"));
    }

    #[test]
    fn test_glob_match_case_sensitive() {
        assert!(glob_match("File.txt", "File.txt"));
        assert!(!glob_match("File.txt", "file.txt"));
    }

    #[test]
    fn test_glob_match_nested_double_star() {
        assert!(glob_match("**/*.rs", "main.rs"));
        assert!(glob_match("**/*.rs", "src/main.rs"));
        assert!(glob_match("**/*.rs", "src/deep/path/main.rs"));
    }

    // -------------------------------------------------------------------------
    // Rule evaluation tests (from comprehensive_tests.rs)
    // -------------------------------------------------------------------------

    use super::super::actions::ActionBuilder;
    use crate::permission::rules::{RuleBuilder, RuleSetBuilder};

    fn make_default_deny_ruleset() -> RuleSet {
        RuleSetBuilder::new()
            .version("1.0")
            .default_file(Effect::Deny)
            .default_command(Effect::Deny)
            .default_network(Effect::Deny)
            .default_inter_agent(Effect::Deny)
            .default_config(Effect::Deny)
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn test_permission_deny_precedence_over_allow() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .rule(
                RuleBuilder::new()
                    .name("deny-cargo-reset")
                    .subject_agent("test-agent")
                    .deny()
                    .action(
                        ActionBuilder::command("cargo")
                            .blocked_args(vec!["reset".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["reset".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_allow_non_blocked_args() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[test]
    fn test_args_match_command_args_any() {
        let engine = PermissionEngine::new(make_default_deny_ruleset());
        assert!(engine.args_match(&CommandArgs::Any, &["any".to_string(), "args".to_string()]));
        assert!(engine.args_match(&CommandArgs::Any, &[]));
    }

    #[test]
    fn test_args_match_command_args_allowed() {
        let engine = PermissionEngine::new(make_default_deny_ruleset());
        let allowed = CommandArgs::Allowed {
            allowed: vec!["build".to_string(), "test".to_string()],
        };
        assert!(engine.args_match(&allowed, &["build".to_string()]));
        assert!(engine.args_match(&allowed, &["build".to_string(), "test".to_string()]));
        assert!(!engine.args_match(&allowed, &["build".to_string(), "run".to_string()]));
    }

    #[test]
    fn test_args_match_command_args_blocked() {
        let engine = PermissionEngine::new(make_default_deny_ruleset());
        let blocked = CommandArgs::Blocked {
            blocked: vec!["reset".to_string(), "--force".to_string()],
        };
        assert!(engine.args_match(&blocked, &["reset".to_string()]));
        assert!(engine.args_match(&blocked, &["reset".to_string(), "--hard".to_string()]));
        assert!(engine.args_match(&blocked, &["commit".to_string(), "--force".to_string()]));
        assert!(!engine.args_match(&blocked, &["commit".to_string(), "push".to_string()]));
    }

    #[tokio::test]
    async fn test_permission_file_op_read_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("read-home")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["/home/**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::FileOp {
            agent: "test-agent".to_string(),
            path: "/home/admin/file.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_file_op_write_denied_by_operation_mismatch() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("read-only")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["/home/**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::FileOp {
            agent: "test-agent".to_string(),
            path: "/home/admin/file.txt".to_string(),
            op: "write".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_command_exec_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_command_exec_denied_command_mismatch() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "git".to_string(),
            args: vec!["status".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_command_args_allowed_list() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo-build-test")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::command("cargo")
                            .allowed_args(vec!["build".to_string(), "test".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string(), "--release".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
        let request = PermissionRequest::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["run".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_command_args_blocked() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo-no-args")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_command_args_any() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-cargo-any-args")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["any".to_string(), "args".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_net_op_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-internal-https")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::network()
                            .with_hosts(vec!["*.internal.corp".to_string()])
                            .with_ports(vec![443])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_network(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::NetOp {
            agent: "test-agent".to_string(),
            host: "api.internal.corp".to_string(),
            port: 443,
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::NetOp {
            agent: "test-agent".to_string(),
            host: "api.internal.corp".to_string(),
            port: 8080,
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_net_op_empty_hosts_matches_all() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-all-ports")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::network().with_ports(vec![443]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_network(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::NetOp {
            agent: "test-agent".to_string(),
            host: "any.host.com".to_string(),
            port: 443,
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_tool_call_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-file-ops")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::tool_call("file_ops")
                            .with_methods(vec!["read".to_string(), "write".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::ToolCall {
            agent: "test-agent".to_string(),
            skill: "file_ops".to_string(),
            method: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::ToolCall {
            agent: "test-agent".to_string(),
            skill: "file_ops".to_string(),
            method: "delete".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_tool_call_empty_methods_matches_all() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-file-ops-any-method")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::tool_call("file_ops").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::ToolCall {
            agent: "test-agent".to_string(),
            skill: "file_ops".to_string(),
            method: "any_method".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_inter_agent_msg_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-to-parent")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::inter_agent()
                            .with_agents(vec!["parent-agent".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_inter_agent(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::InterAgentMsg {
            from: "test-agent".to_string(),
            to: "parent-agent".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::InterAgentMsg {
            from: "test-agent".to_string(),
            to: "stranger-agent".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_inter_agent_empty_agents_matches_all() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-all-inter-agent")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::inter_agent().build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_inter_agent(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::InterAgentMsg {
            from: "test-agent".to_string(),
            to: "any-agent".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_config_write_allowed() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-config-write")
                    .subject_agent("test-agent")
                    .allow()
                    .action(
                        ActionBuilder::config_write()
                            .with_files(vec!["configs/*.json".to_string()])
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .default_config(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::ConfigWrite {
            agent: "test-agent".to_string(),
            config_file: "configs/agents.json".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::ConfigWrite {
            agent: "test-agent".to_string(),
            config_file: "secrets/passwords.json".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_config_write_empty_files_matches_all() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-all-config-write")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::config_write().build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_config(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::ConfigWrite {
            agent: "test-agent".to_string(),
            config_file: "any/config.json".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_subject_exact_match() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("exact-match")
                    .subject_agent("specific-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::FileOp {
            agent: "specific-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::FileOp {
            agent: "other-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_subject_glob_match() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("exact-match")
                    .subject_agent("specific-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::FileOp {
            agent: "specific-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_unknown_agent_uses_defaults() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .default_file(Effect::Allow)
            .default_command(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::FileOp {
            agent: "totally-unknown-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
        let request = PermissionRequest::CommandExec {
            agent: "totally-unknown-agent".to_string(),
            cmd: "ls".to_string(),
            args: vec![],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_empty_ruleset() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::FileOp {
            agent: "any-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_rule_action_type_mismatch() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("command-only")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::FileOp {
            agent: "test-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { .. }));
    }

    #[tokio::test]
    async fn test_permission_unicode_in_path() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-unicode")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["/home/**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Deny)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::FileOp {
            agent: "test-agent".to_string(),
            path: "/home/\u{7528}\u{6237}/\u{6587}\u{4EF6}.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Allowed { .. }));
    }

    #[tokio::test]
    async fn test_permission_denied_reason_includes_rule_name() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("my-specific-deny-rule")
                    .subject_agent("test-agent")
                    .deny()
                    .action(ActionBuilder::file("read", vec!["/secret/**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Allow)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::FileOp {
            agent: "test-agent".to_string(),
            path: "/secret/file.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        if let PermissionResponse::Denied { reason, rule } = response {
            assert!(rule == "my-specific-deny-rule");
            assert!(reason.contains("my-specific-deny-rule"));
        } else {
            panic!("Expected Denied response");
        }
    }

    #[tokio::test]
    async fn test_permission_allowed_token_format() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("allow-all")
                    .subject_agent("test-agent")
                    .allow()
                    .action(ActionBuilder::file("read", vec!["**".to_string()]).build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_file(Effect::Allow)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::FileOp {
            agent: "test-agent".to_string(),
            path: "/any/path.txt".to_string(),
            op: "read".to_string(),
        };
        let response = engine.evaluate(request).await;
        if let PermissionResponse::Allowed { token } = response {
            assert!(token.starts_with("perm_"));
        } else {
            panic!("Expected Allowed response");
        }
    }

    #[tokio::test]
    async fn test_permission_multiple_deny_rules_first_wins() {
        let ruleset = RuleSetBuilder::new()
            .version("1.0")
            .rule(
                RuleBuilder::new()
                    .name("deny-all-cargo")
                    .subject_agent("multi-deny-agent")
                    .deny()
                    .action(ActionBuilder::command("cargo").build().unwrap())
                    .build()
                    .unwrap(),
            )
            .default_command(Effect::Allow)
            .build()
            .unwrap();
        let engine = PermissionEngine::new(ruleset);
        let request = PermissionRequest::CommandExec {
            agent: "multi-deny-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        };
        let response = engine.evaluate(request).await;
        assert!(matches!(response, PermissionResponse::Denied { rule, .. } if rule == "deny-all-cargo"));
    }

    #[test]
    fn test_subject_matches_unicode() {
        let subject = Subject {
            agent: "\u{65E5}\u{672C}\u{8A9E}-agent".to_string(),
            match_type: MatchType::Exact,
        };
        assert!(subject.matches("\u{65E5}\u{672C}\u{8A9E}-agent"));
        assert!(!subject.matches("other-agent"));
    }

    #[test]
    fn test_subject_matches_glob_unicode() {
        let subject = Subject {
            agent: "*-agent".to_string(),
            match_type: MatchType::Glob,
        };
        assert!(subject.matches("\u{65E5}\u{672C}\u{8A9E}-agent"));
        assert!(subject.matches("test-agent"));
        assert!(!subject.matches("agent"));
    }
}

impl Rule {
    /// Parse subject from string (for testing)
    pub fn parse_subject(agent: &str) -> Subject {
        Subject {
            agent: agent.to_string(),
            match_type: MatchType::Exact,
        }
    }
    
    /// Parse subject with match type
    pub fn parse_subject_with_match(agent: &str, match_type: &str) -> Subject {
        Subject {
            agent: agent.to_string(),
            match_type: match match_type {
                "glob" => MatchType::Glob,
                _ => MatchType::Exact,
            },
        }
    }
}
