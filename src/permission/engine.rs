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
    fn args_match(&self, rule_args: &CommandArgs, request_args: &[String]) -> bool {
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
fn glob_match(pattern: &str, text: &str) -> bool {
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
