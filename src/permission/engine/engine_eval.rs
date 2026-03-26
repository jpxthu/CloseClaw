//! Permission Engine - Evaluation logic.

use super::engine_matching::action_matches_request;
use super::engine_types::{
    Action, Defaults, Effect, PermissionRequest, PermissionRequestBody, PermissionResponse, Rule,
    RuleSet, Subject,
};
use std::collections::HashMap;
use tracing::info;

/// Permission Engine - evaluates access requests against rules
pub struct PermissionEngine {
    /// RuleSet
    rules: RuleSet,
    /// O(1) lookup index: agent_id -> list of rule indices
    agent_rule_index: HashMap<String, Vec<usize>>,
    /// O(1) lookup index: "{user_id}:{agent_id}" -> list of rule indices
    user_agent_rule_index: HashMap<String, Vec<usize>>,
    /// Loaded templates: name -> Template
    templates: HashMap<String, crate::permission::templates::Template>,
}

impl PermissionEngine {
    /// Create a new PermissionEngine from a RuleSet
    pub fn new(rules: RuleSet) -> Self {
        let mut engine = Self {
            rules: rules.clone(),
            agent_rule_index: HashMap::new(),
            user_agent_rule_index: HashMap::new(),
            templates: HashMap::new(),
        };
        let agent_index: HashMap<String, Vec<usize>> = HashMap::new();
        let user_agent_index: HashMap<String, Vec<usize>> = HashMap::new();
        engine.agent_rule_index = agent_index;
        engine.user_agent_rule_index = user_agent_index;
        for (idx, rule) in rules.rules.iter().enumerate() {
            match &rule.subject {
                Subject::AgentOnly { agent, .. } => {
                    engine
                        .agent_rule_index
                        .entry(agent.clone())
                        .or_default()
                        .push(idx);
                }
                Subject::UserAndAgent { user_id, agent, .. } => {
                    let key = format!("{}:{}", user_id, agent);
                    engine
                        .user_agent_rule_index
                        .entry(key)
                        .or_default()
                        .push(idx);
                    engine
                        .agent_rule_index
                        .entry(agent.clone())
                        .or_default()
                        .push(idx);
                }
            }
        }
        engine
    }

    /// Rebuild the lookup indices from a given ruleset (sync helper).
    pub fn rebuild_indices_with_rules(&mut self, rules: &RuleSet) {
        let mut agent_index: HashMap<String, Vec<usize>> = HashMap::new();
        let mut user_agent_index: HashMap<String, Vec<usize>> = HashMap::new();

        for (idx, rule) in rules.rules.iter().enumerate() {
            match &rule.subject {
                Subject::AgentOnly { agent, .. } => {
                    agent_index.entry(agent.clone()).or_default().push(idx);
                }
                Subject::UserAndAgent { user_id, agent, .. } => {
                    let key = format!("{}:{}", user_id, agent);
                    user_agent_index.entry(key).or_default().push(idx);
                    agent_index.entry(agent.clone()).or_default().push(idx);
                }
            }
        }

        self.agent_rule_index = agent_index;
        self.user_agent_rule_index = user_agent_index;
    }

    /// Reload rules from a new RuleSet
    pub fn reload_rules(&mut self, rules: RuleSet) {
        self.rebuild_indices_with_rules(&rules);
        self.rules = rules;
    }

    /// Load templates into the engine
    pub fn load_templates(
        &mut self,
        templates: HashMap<String, crate::permission::templates::Template>,
    ) {
        self.templates = templates;
    }

    /// Simplified permission check — evaluates if `agent_id` may perform `action`.
    ///
    /// `action` is one of: "exec", "file_read", "file_write", "network",
    /// "spawn", "tool_call", "config_write".
    ///
    /// Uses a bare request (no caller user context) and wildcard arguments
    /// to check the coarse-grained permission.
    pub fn check(&self, agent_id: &str, action: &str) -> PermissionResponse {
        let body = match action {
            "exec" => PermissionRequestBody::CommandExec {
                agent: agent_id.to_string(),
                cmd: "*".to_string(),
                args: Vec::new(),
            },
            "file_read" => PermissionRequestBody::FileOp {
                agent: agent_id.to_string(),
                path: "*".to_string(),
                op: "read".to_string(),
            },
            "file_write" => PermissionRequestBody::FileOp {
                agent: agent_id.to_string(),
                path: "*".to_string(),
                op: "write".to_string(),
            },
            "network" => PermissionRequestBody::NetOp {
                agent: agent_id.to_string(),
                host: "*".to_string(),
                port: 0,
            },
            "spawn" => PermissionRequestBody::InterAgentMsg {
                from: agent_id.to_string(),
                to: "*".to_string(),
            },
            "tool_call" => PermissionRequestBody::ToolCall {
                agent: agent_id.to_string(),
                skill: "*".to_string(),
                method: "*".to_string(),
            },
            "config_write" => PermissionRequestBody::ConfigWrite {
                agent: agent_id.to_string(),
                config_file: "*".to_string(),
            },
            // Unknown action — deny by default
            _ => {
                return PermissionResponse::Denied {
                    reason: format!("unknown action: {}", action),
                    rule: "<check>".to_string(),
                };
            }
        };

        self.evaluate(PermissionRequest::Bare(body))
    }

    /// Evaluate a permission request.
    pub fn evaluate(&self, request: PermissionRequest) -> PermissionResponse {
        let caller = request.caller();
        let agent_id = caller.agent.clone();

        info!(
            agent = %agent_id,
            user_id = %caller.user_id,
            request_type = ?request.body(),
            "permission check initiated"
        );

        let rules = self.rules.clone();

        // ---- Step 0: Creator Rule (highest priority, short-circuit return) ----
        let effective_creator_id = if !caller.creator_id.is_empty() {
            Some(caller.creator_id.as_str())
        } else {
            rules.agent_creators.get(&agent_id).map(|s| s.as_str())
        };
        if let Some(creator_id) = effective_creator_id {
            if caller.user_id == creator_id {
                info!(agent = %agent_id, result = "allowed", reason = "creator_rule", "permission check completed");
                return PermissionResponse::Allowed {
                    token: generate_token(),
                };
            }
        }

        // ---- Step 1: Build candidate rule list ----
        let mut candidates: Vec<usize> = Vec::new();

        // 1a. User+Agent dual-key index lookup (O(1))
        let index_key = format!("{}:{}", caller.user_id, agent_id);
        if let Some(indices) = self.user_agent_rule_index.get(&index_key) {
            candidates.extend(indices);
        }

        // 1b. Agent-only index lookup (O(1))
        if let Some(indices) = self.agent_rule_index.get(&agent_id) {
            candidates.extend(indices);
        }

        // 1c. Glob fallback (only if 1a and 1b produced nothing)
        if candidates.is_empty() {
            for (idx, rule) in rules.rules.iter().enumerate() {
                if rule.subject.matches(&caller) {
                    candidates.push(idx);
                }
            }
        }

        // ---- Step 2: Sort by priority (desc) ----
        candidates.sort_by(|&a, &b| rules.rules[b].priority.cmp(&rules.rules[a].priority));

        // ---- Step 3: Expand templates ----
        let (expanded_rules, expanded_indices) = self.expand_templates_sync(&candidates, &rules);

        // ---- Step 4: Evaluate (AWS IAM-style deny-precedence) ----
        let mut matching_rule_name: Option<String> = None;
        for &rule_idx in &expanded_indices {
            let rule = &expanded_rules[rule_idx];

            if !rule.subject.matches(&caller) {
                continue;
            }

            if !self.rule_actions_match(rule, request.body()) {
                continue;
            }

            matching_rule_name = Some(rule.name.clone());

            if rule.effect == Effect::Deny {
                let reason = format!("action denied by rule '{}'", rule.name);
                info!(agent = %agent_id, result = "denied", rule = %rule.name, "permission check completed");
                return PermissionResponse::Denied {
                    reason,
                    rule: rule.name.clone(),
                };
            }
        }

        // No deny found; if any rule matched, allow
        if matching_rule_name.is_some() {
            info!(agent = %agent_id, result = "allowed", reason = "matched_rule", "permission check completed");
            return PermissionResponse::Allowed {
                token: generate_token(),
            };
        }

        // ---- Step 5: Default fallback ----
        let response = self.default_deny(request.body(), &rules.defaults, "no matching rule");
        info!(
            agent = %agent_id,
            result = %match &response {
                PermissionResponse::Allowed { .. } => "allowed",
                PermissionResponse::Denied { .. } => "denied",
            },
            reason = "default_fallback",
            "permission check completed"
        );
        response
    }

    /// Expand template references in candidate rules.
    ///
    /// For template-based rules, replaces the rule with one pseudo-rule per resolved
    /// template action (each pseudo-rule carries a single action, allowing precise
    /// deny/action matching). For non-template rules, keeps the rule as-is.
    fn expand_templates_sync(
        &self,
        candidates: &[usize],
        ruleset: &RuleSet,
    ) -> (Vec<Rule>, Vec<usize>) {
        let mut expanded_rules: Vec<Rule> = Vec::new();
        let mut expanded_indices: Vec<usize> = Vec::new();

        for &idx in candidates {
            let rule = &ruleset.rules[idx];

            if let Some(ref template_ref) = rule.template {
                if let Some(tmpl) = self.templates.get(&template_ref.name) {
                    let actions = resolve_template_actions(tmpl, &template_ref.overrides);
                    for action in actions {
                        let pseudo_rule = Rule {
                            name: rule.name.clone(),
                            subject: rule.subject.clone(),
                            effect: rule.effect,
                            actions: vec![action],
                            template: None,
                            priority: rule.priority,
                        };
                        expanded_indices.push(expanded_rules.len());
                        expanded_rules.push(pseudo_rule);
                    }
                }
            } else {
                expanded_indices.push(expanded_rules.len());
                expanded_rules.push(rule.clone());
            }
        }

        // Deduplicate while preserving order
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut unique_indices: Vec<usize> = Vec::new();
        for &idx in &expanded_indices {
            if seen.insert(idx) {
                unique_indices.push(idx);
            }
        }

        (expanded_rules, unique_indices)
    }

    /// Get default action when no rule matches.
    fn default_deny(
        &self,
        request: &PermissionRequestBody,
        defaults: &Defaults,
        reason: &str,
    ) -> PermissionResponse {
        let effect = match request {
            PermissionRequestBody::FileOp { .. } => defaults.file,
            PermissionRequestBody::CommandExec { .. } => defaults.command,
            PermissionRequestBody::NetOp { .. } => defaults.network,
            PermissionRequestBody::InterAgentMsg { .. } => defaults.inter_agent,
            PermissionRequestBody::ConfigWrite { .. } => defaults.config,
            PermissionRequestBody::ToolCall { .. } => defaults.file,
        };

        match effect {
            Effect::Allow => PermissionResponse::Allowed {
                token: generate_token(),
            },
            Effect::Deny => PermissionResponse::Denied {
                reason: reason.to_string(),
                rule: "default".to_string(),
            },
        }
    }

    /// Check if a rule's actions match the request.
    fn rule_actions_match(&self, rule: &Rule, request: &PermissionRequestBody) -> bool {
        let actions = if let Some(ref template_ref) = rule.template {
            if let Some(tmpl) = self.templates.get(&template_ref.name) {
                resolve_template_actions(tmpl, &template_ref.overrides)
            } else {
                rule.actions.clone()
            }
        } else {
            rule.actions.clone()
        };

        for action in &actions {
            if action_matches_request(action, request) {
                return true;
            }
        }
        false
    }
}

/// Resolve template actions with overrides applied.
fn resolve_template_actions(
    tmpl: &crate::permission::templates::Template,
    overrides: &HashMap<String, serde_json::Value>,
) -> Vec<Action> {
    if let Some(overridden_actions) = overrides.get("actions") {
        if let Ok(actions) = serde_json::from_value(overridden_actions.clone()) {
            return actions;
        }
    }
    tmpl.actions.clone()
}

/// Generate a short-lived permission token.
fn generate_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("perm_{}_{:016x}", duration.as_secs(), rand_u64())
}

fn rand_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    RandomState::new().build_hasher().finish()
}
