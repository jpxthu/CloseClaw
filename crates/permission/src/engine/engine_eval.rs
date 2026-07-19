//! Permission Engine - Evaluation logic.

use super::engine_helpers::{generate_token, get_agent_deny_subjects, resolve_template_actions};
use super::engine_matching::action_matches_request;
use super::engine_risk::assess_risk_level;
use super::engine_types::{
    Defaults, Effect, PermissionRequest, PermissionRequestBody, PermissionResponse, Rule, RuleSet,
    Subject,
};
use super::engine_workspace;
use super::rejection_log::{build_rejection_log, RejectionLogger};
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::session_mode_query::SessionModeQuery;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;
// NOTE: Cache fields (agent_permissions, user_effective_permissions) removed per
// design doc: "权限评估每次新鲜计算，不缓存评估结果"

/// Build O(1) lookup indices from a RuleSet.
///
/// Returns `(agent_rule_index, user_agent_rule_index)` used for fast
/// candidate collection during evaluation.
pub(crate) fn build_rule_indices(
    rules: &RuleSet,
) -> (HashMap<String, Vec<usize>>, HashMap<String, Vec<usize>>) {
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

    (agent_index, user_agent_index)
}

/// Permission Engine - evaluates access requests against rules
pub struct PermissionEngine {
    /// RuleSet
    pub(crate) rules: RuleSet,
    /// O(1) lookup index: agent_id -> list of rule indices
    agent_rule_index: HashMap<String, Vec<usize>>,
    /// O(1) lookup index: "{user_id}:{agent_id}" -> list of rule indices
    user_agent_rule_index: HashMap<String, Vec<usize>>,
    /// Loaded templates: name -> Template
    templates: HashMap<String, crate::templates::Template>,
    /// Data root directory for workspace path resolution
    data_root: PathBuf,
    /// Optional session mode query for mode-aware filtering.
    /// When set, `evaluate` will consult the agent's session mode
    /// for additional access-control decisions.
    session_mode_query: Option<Arc<dyn SessionModeQuery>>,
    /// Optional rejection logger. When set and `evaluate` returns `Denied`,
    /// a structured rejection log entry is recorded.
    rejection_logger: Option<Arc<dyn RejectionLogger>>,
}

// --- Construction & index management ---

impl PermissionEngine {
    /// Create a new PermissionEngine from a RuleSet
    pub fn new(mut rules: RuleSet, data_root: PathBuf) -> Self {
        rules.compute_version();
        let mut engine = Self {
            rules: rules.clone(),
            agent_rule_index: HashMap::new(),
            user_agent_rule_index: HashMap::new(),
            templates: HashMap::new(),
            data_root,
            session_mode_query: None,
            rejection_logger: None,
        };
        engine.rebuild_indices_with_rules(&rules);
        engine
    }

    /// Create a new PermissionEngine with a default data root (for tests)
    pub fn new_with_default_data_root(rules: RuleSet) -> Self {
        Self::new(rules, PathBuf::from("/tmp/closeclaw_test"))
    }

    /// Rebuild the lookup indices from a given ruleset (sync helper).
    pub fn rebuild_indices_with_rules(&mut self, rules: &RuleSet) {
        let (agent_index, user_agent_index) = build_rule_indices(rules);
        self.agent_rule_index = agent_index;
        self.user_agent_rule_index = user_agent_index;
    }

    /// Reload rules from a new RuleSet
    pub fn reload_rules(&mut self, mut rules: RuleSet) {
        rules.compute_version();
        self.rebuild_indices_with_rules(&rules);
        self.rules = rules;
    }

    /// Get a reference to the current ruleset.
    pub fn rules(&self) -> &RuleSet {
        &self.rules
    }

    /// Load templates into the engine
    pub fn load_templates(&mut self, templates: HashMap<String, crate::templates::Template>) {
        self.templates = templates;
    }

    /// Inject a session mode query for mode-aware permission evaluation.
    ///
    /// When provided, `evaluate` will look up the agent's current
    /// `SessionMode` and apply mode-specific access rules.
    pub fn with_session_mode_query(mut self, query: Arc<dyn SessionModeQuery>) -> Self {
        self.session_mode_query = Some(query);
        self
    }

    /// Get a reference to the session mode query, if set.
    pub fn session_mode_query(&self) -> Option<&Arc<dyn SessionModeQuery>> {
        self.session_mode_query.as_ref()
    }

    /// Inject a rejection logger for recording denied permission requests.
    pub fn with_rejection_logger(mut self, logger: Arc<dyn RejectionLogger>) -> Self {
        self.rejection_logger = Some(logger);
        self
    }

    /// Get a reference to the rejection logger, if set.
    pub fn rejection_logger(&self) -> Option<&Arc<dyn RejectionLogger>> {
        self.rejection_logger.as_ref()
    }

    /// Log a rejection if the logger is set, the response is `Denied`,
    /// and the session is in Auto Mode.
    ///
    /// Per design doc: rejection logs are only generated for Auto Mode
    /// sessions (Plan/Normal/unknown modes do not produce logs).
    fn log_rejection(&self, response: &PermissionResponse, body: &PermissionRequestBody) {
        if let Some(logger) = &self.rejection_logger {
            if let PermissionResponse::Denied {
                reason, risk_level, ..
            } = response
            {
                // Determine session mode from query (best-effort).
                let session_mode = self
                    .session_mode_query
                    .as_ref()
                    .and_then(|q| q.get_session_mode(body.agent_id()));
                // Only record rejection logs in Auto Mode.
                if session_mode != Some(SessionMode::Auto) {
                    return;
                }
                let entry = build_rejection_log(body, reason.clone(), *risk_level, session_mode);
                logger.log(&entry);
            }
        }
    }
}

// --- Evaluation & helpers ---

impl PermissionEngine {
    /// Evaluate a permission request using the engine's current rules.
    pub fn evaluate(
        &self,
        request: PermissionRequest,
        extra_deny_subjects: Option<Vec<Subject>>,
    ) -> PermissionResponse {
        self.evaluate_inner(
            request,
            extra_deny_subjects,
            &self.rules,
            &self.agent_rule_index,
            &self.user_agent_rule_index,
        )
    }

    /// Evaluate a permission request using an external rule set.
    ///
    /// Builds temporary O(1) indices from the provided `rules` and delegates
    /// to the same evaluation logic as `evaluate()`. This allows re-evaluation
    /// against a snapshot of rules (e.g., for approval re-evaluation).
    pub fn evaluate_with_rules(
        &self,
        request: PermissionRequest,
        extra_deny_subjects: Option<Vec<Subject>>,
        rules: &RuleSet,
    ) -> PermissionResponse {
        let (agent_index, user_agent_index) = build_rule_indices(rules);
        self.evaluate_inner(
            request,
            extra_deny_subjects,
            rules,
            &agent_index,
            &user_agent_index,
        )
    }

    /// Core evaluation logic shared by `evaluate` and `evaluate_with_rules`.
    ///
    /// `agent_rule_index` and `user_agent_rule_index` provide O(1) lookup
    /// for candidate collection — either from the engine's own cache or from
    /// temporary indices built for an external rule set.
    fn evaluate_inner(
        &self,
        request: PermissionRequest,
        extra_deny_subjects: Option<Vec<Subject>>,
        rules: &RuleSet,
        agent_rule_index: &HashMap<String, Vec<usize>>,
        user_agent_rule_index: &HashMap<String, Vec<usize>>,
    ) -> PermissionResponse {
        let caller = request.caller();
        let agent_id = caller.agent.clone();

        // Step 0: Plan Mode write-operation filtering
        if let Some(denied) = self.check_plan_mode_filter(&request, &agent_id) {
            self.log_rejection(&denied, request.body());
            return denied;
        }

        // Step 0.4: Config dir forced deny (hardcoded rule)
        // Permission config directory access is unconditionally denied for
        // agents, regardless of rules or defaults.
        if let PermissionRequestBody::FileOp { op, path, .. } = request.body() {
            if (op == "read" || op == "write")
                && engine_workspace::is_config_dir_path(&self.data_root, path)
            {
                info!(
                    agent = %agent_id,
                    result = "denied",
                    reason = "config_dir_forced_deny",
                    path = %path,
                    "permission check completed"
                );
                return PermissionResponse::Denied {
                    reason: "config directory access denied by hardcoded rule".to_string(),
                    rule: "<config_dir_guard>".to_string(),
                    risk_level: assess_risk_level(request.body()),
                };
            }
        }

        // Step 0.5: Workspace forced authorization
        if let PermissionRequestBody::FileOp { agent, path, op } = request.body() {
            if (op == "read" || op == "write")
                && engine_workspace::is_workspace_path(
                    &self.data_root,
                    agent,
                    &caller.user_id,
                    path,
                )
            {
                info!(
                    agent = %agent_id,
                    result = "allowed",
                    reason = "workspace_forced_auth",
                    "permission check completed"
                );
                return PermissionResponse::Allowed {
                    token: generate_token(),
                    context_modifier: None,
                };
            }
        }

        info!(
            agent = %agent_id,
            user_id = %caller.user_id,
            request_type = ?request.body(),
            "permission check initiated"
        );

        let is_owner = caller.user_id == "owner";

        // Step 0: Creator rule (highest priority)
        if let Some(response) = self.check_creator_rule(&caller, &agent_id) {
            return response;
        }

        // Step 1: Agent phase — collect AgentOnly candidates and evaluate
        let agent_candidates =
            self.collect_agent_candidates_with_index(&caller, &agent_id, rules, agent_rule_index);
        let agent_result = self.match_rules(&agent_candidates, rules, &caller, request.body());

        // Step 1.4: ConfigWrite Allowed → forced Denied
        // Design doc: "此维度永远高危，只能走单次审批，不能被加入白名单"
        let agent_result = match agent_result {
            Some(PermissionResponse::Allowed { .. })
                if matches!(request.body(), PermissionRequestBody::ConfigWrite { .. }) =>
            {
                info!(
                    agent = %agent_id,
                    result = "denied",
                    reason = "config_write_forced_deny",
                    "permission check completed"
                );
                Some(PermissionResponse::Denied {
                    reason: "config write cannot be whitelisted, only single approval".to_string(),
                    rule: "<config_write_guard>".to_string(),
                    risk_level: assess_risk_level(request.body()),
                })
            }
            other => other,
        };

        // Owner shortcut: skip User phase entirely, Agent result is final
        if is_owner {
            let response = agent_result.unwrap_or_else(|| {
                self.default_response(request.body(), &rules.defaults, "no matching rule")
            });
            self.log_rejection(&response, request.body());
            info!(
                agent = %agent_id,
                result = %match &response {
                    PermissionResponse::Allowed { .. } => "allowed",
                    PermissionResponse::Denied { .. } => "denied",
                },
                reason = "owner_shortcut",
                "permission check completed"
            );
            return response;
        }

        // Step 2: User phase — collect UserAndAgent candidates and evaluate
        let user_candidates = self.collect_user_agent_candidates_with_index(
            &caller,
            &agent_id,
            rules,
            user_agent_rule_index,
        );
        let user_result = self.match_rules(&user_candidates, rules, &caller, request.body());

        // Step 1.4: ConfigWrite Allowed → forced Denied (user phase)
        let user_result = match user_result {
            Some(PermissionResponse::Allowed { .. })
                if matches!(request.body(), PermissionRequestBody::ConfigWrite { .. }) =>
            {
                info!(
                    agent = %agent_id,
                    result = "denied",
                    reason = "config_write_forced_deny",
                    "permission check completed"
                );
                Some(PermissionResponse::Denied {
                    reason: "config write cannot be whitelisted, only single approval".to_string(),
                    rule: "<config_write_guard>".to_string(),
                    risk_level: assess_risk_level(request.body()),
                })
            }
            other => other,
        };

        // Step 3: Merge results (two-phase logic)
        let response = match (agent_result, user_result) {
            (Some(PermissionResponse::Denied { .. }), _) => PermissionResponse::Denied {
                reason: "action denied by agent rule".to_string(),
                rule: "<agent_phase>".to_string(),
                risk_level: assess_risk_level(request.body()),
            },
            (_, Some(PermissionResponse::Denied { .. })) => PermissionResponse::Denied {
                reason: "action denied by user rule".to_string(),
                rule: "<user_phase>".to_string(),
                risk_level: assess_risk_level(request.body()),
            },
            (
                Some(PermissionResponse::Allowed { .. }),
                Some(PermissionResponse::Allowed { .. }),
            ) => PermissionResponse::Allowed {
                token: generate_token(),
                context_modifier: None,
            },
            // Agent allowed, no user rule → agent result wins
            // (when user_id is empty, user phase is effectively skipped)
            (Some(PermissionResponse::Allowed { .. }), None) if caller.user_id.is_empty() => {
                PermissionResponse::Allowed {
                    token: generate_token(),
                    context_modifier: None,
                }
            }
            _ => {
                // Non-Owner user with user_id: use user_defaults (all Deny)
                // Empty user_id / system caller: use defaults (Agent defaults)
                let defaults_ref = if !caller.user_id.is_empty() {
                    &rules.user_defaults
                } else {
                    &rules.defaults
                };
                self.default_response(request.body(), defaults_ref, "no matching rule")
            }
        };
        self.log_rejection(&response, request.body());
        info!(
            agent = %agent_id,
            result = %match &response {
                PermissionResponse::Allowed { .. } => "allowed",
                PermissionResponse::Denied { .. } => "denied",
            },
            reason = "two_phase_merge",
            "permission check completed"
        );

        // Step 9: Extra Deny — override with deny if caller matches any extra deny subject
        if let Some(extra_subjects) = extra_deny_subjects {
            for subject in &extra_subjects {
                if subject.matches(&caller) {
                    info!(
                        agent = %agent_id,
                        result = "denied",
                        reason = "extra_deny",
                        "permission check completed"
                    );
                    let extra_denied = PermissionResponse::Denied {
                        reason: "action denied by parent agent restriction".to_string(),
                        rule: "<extra_deny>".to_string(),
                        risk_level: assess_risk_level(request.body()),
                    };
                    self.log_rejection(&extra_denied, request.body());
                    return extra_denied;
                }
            }
        }

        response
    }

    /// Plan Mode write-operation filtering.
    ///
    /// When the agent's session mode is `Plan`, the following operations are
    /// denied:
    /// - `FileOp` with op = "write" (unless the path is under plans/)
    /// - `CommandExec`
    /// - `ConfigWrite`
    ///
    /// The plans/ directory check: path starts with "plans/" or contains "/plans/".
    ///
    /// Returns `Some(Denied)` if the operation should be blocked, `None` to
    /// proceed with normal evaluation.
    ///
    /// # Design Note: Runtime Evaluation vs. Static Binding
    ///
    /// The design doc (`docs/design/mode/README.md`) states that tool filtering
    /// and permission boundaries are "determined at session creation and statically
    /// effective" ("静态生效"). This method, however, evaluates the session mode
    /// at runtime on every permission request via `session_mode_query`.
    ///
    /// This is intentional: runtime evaluation ensures that when the session mode
    /// changes (e.g., Plan → Auto via `/execute`), the new permission set takes
    /// effect immediately without requiring session reconstruction. The alternative
    /// — static binding at session creation — would introduce a stale-tool-set risk
    /// where mode switches leave the old tool filter active until the session is
    /// torn down and rebuilt.
    ///
    /// The doc's "静态生效" describes the *behavioral* contract: for any given
    /// mode, the set of allowed/denied tools is deterministic and does not vary
    /// within a single permission check. It does not prescribe the *implementation*
    /// mechanism. Runtime lookup satisfies this contract while also supporting
    /// seamless mode transitions.
    fn check_plan_mode_filter(
        &self,
        request: &PermissionRequest,
        agent_id: &str,
    ) -> Option<PermissionResponse> {
        let query = self.session_mode_query.as_ref()?;
        let mode = query.get_session_mode(agent_id)?;
        if mode != SessionMode::Plan {
            return None;
        }

        let body = request.body();
        match body {
            PermissionRequestBody::FileOp { op, path, .. } if op == "write" => {
                if is_plans_path(path) {
                    return None;
                }
                info!(
                    agent = agent_id,
                    result = "denied",
                    reason = "plan_mode_write_denied",
                    path = %path,
                    "permission check completed"
                );
                Some(PermissionResponse::Denied {
                    reason: "write operation denied in Plan mode".to_string(),
                    rule: "<plan_mode_filter>".to_string(),
                    risk_level: assess_risk_level(body),
                })
            }
            PermissionRequestBody::CommandExec { .. } => {
                info!(
                    agent = agent_id,
                    result = "denied",
                    reason = "plan_mode_command_denied",
                    "permission check completed"
                );
                Some(PermissionResponse::Denied {
                    reason: "command execution denied in Plan mode".to_string(),
                    rule: "<plan_mode_filter>".to_string(),
                    risk_level: assess_risk_level(body),
                })
            }
            PermissionRequestBody::ConfigWrite { .. } => {
                info!(
                    agent = agent_id,
                    result = "denied",
                    reason = "plan_mode_config_write_denied",
                    "permission check completed"
                );
                Some(PermissionResponse::Denied {
                    reason: "config write denied in Plan mode".to_string(),
                    rule: "<plan_mode_filter>".to_string(),
                    risk_level: assess_risk_level(body),
                })
            }
            // AskUserQuestion: allow for clarification, but inject context
            // marker so the agent knows it cannot be used as an approval
            // substitute (design doc: "禁止用 AskUserQuestion 替代审批").
            PermissionRequestBody::ToolCall { skill, .. } if skill == "ask_user_question" => {
                info!(
                    agent = agent_id,
                    result = "allowed_with_context",
                    reason = "plan_mode_ask_user_question_clarification_only",
                    "permission check completed"
                );
                Some(PermissionResponse::Allowed {
                    token: generate_token(),
                    context_modifier: Some(
                        "[plan_mode_context] AskUserQuestion is for requirement \
                         clarification only. Do NOT use it as an approval \
                         substitute."
                            .to_string(),
                    ),
                })
            }
            _ => None,
        }
    }

    /// Step 0: Check creator rule — if the caller is the agent's creator, allow immediately.
    fn check_creator_rule(
        &self,
        caller: &super::engine_types::Caller,
        agent_id: &str,
    ) -> Option<PermissionResponse> {
        let effective_creator_id = if !caller.creator_id.is_empty() {
            Some(caller.creator_id.as_str())
        } else {
            self.rules.agent_creators.get(agent_id).map(|s| s.as_str())
        };

        if let Some(creator_id) = effective_creator_id {
            if caller.user_id == creator_id {
                info!(
                    agent = %agent_id,
                    result = "allowed",
                    reason = "creator_rule",
                    "permission check completed"
                );
                return Some(PermissionResponse::Allowed {
                    token: generate_token(),
                    context_modifier: None,
                });
            }
        }
        None
    }

    /// Extract AgentOnly + Deny subjects from parent agent, replacing agent with child_agent_id.
    /// Used for sub-agent permission inheritance via parent-agent deny propagation.
    pub fn get_agent_deny_subjects(
        &self,
        parent_agent_id: &str,
        child_agent_id: &str,
    ) -> Vec<Subject> {
        get_agent_deny_subjects(&self.rules, parent_agent_id, child_agent_id)
    }
}

// --- Candidate collection & rule matching ---

impl PermissionEngine {
    /// Collect Subject::AgentOnly candidate rule indices via provided index (O(1)),
    /// then via Glob fallback if no exact match (matches AgentOnly only).
    fn collect_agent_candidates_with_index(
        &self,
        caller: &super::engine_types::Caller,
        agent_id: &str,
        rules: &RuleSet,
        agent_rule_index: &HashMap<String, Vec<usize>>,
    ) -> Vec<usize> {
        let mut candidates: Vec<usize> = Vec::new();

        if let Some(indices) = agent_rule_index.get(agent_id) {
            let filtered = indices
                .iter()
                .filter(|&&idx| rules.rules[idx].subject.is_agent_only())
                .copied();
            candidates.extend(filtered);
        }

        if candidates.is_empty() {
            for (idx, rule) in rules.rules.iter().enumerate() {
                if rule.subject.is_agent_only() && rule.subject.matches(caller) {
                    candidates.push(idx);
                }
            }
        }

        candidates.sort_by(|&a, &b| rules.rules[b].priority.cmp(&rules.rules[a].priority));
        candidates
    }

    /// Collect Subject::UserAndAgent candidate rule indices via provided index (O(1)),
    /// then via Glob fallback if no exact match (matches UserAndAgent only).
    pub(crate) fn collect_user_agent_candidates_with_index(
        &self,
        caller: &super::engine_types::Caller,
        agent_id: &str,
        rules: &RuleSet,
        user_agent_rule_index: &HashMap<String, Vec<usize>>,
    ) -> Vec<usize> {
        let mut candidates: Vec<usize> = Vec::new();

        let index_key = format!("{}:{}", caller.user_id, agent_id);
        if let Some(indices) = user_agent_rule_index.get(&index_key) {
            candidates.extend(indices);
        }

        if candidates.is_empty() {
            for (idx, rule) in rules.rules.iter().enumerate() {
                if rule.subject.is_user_and_agent() && rule.subject.matches(caller) {
                    candidates.push(idx);
                }
            }
        }

        candidates.sort_by(|&a, &b| rules.rules[b].priority.cmp(&rules.rules[a].priority));
        candidates
    }

    /// Collect Subject::UserAndAgent candidate rule indices via engine's own index.
    pub(crate) fn collect_user_agent_candidates(
        &self,
        caller: &super::engine_types::Caller,
        agent_id: &str,
        rules: &RuleSet,
    ) -> Vec<usize> {
        self.collect_user_agent_candidates_with_index(
            caller,
            agent_id,
            rules,
            &self.user_agent_rule_index,
        )
    }

    /// Steps 3-4: Expand templates, then evaluate rules (deny-precedence).
    pub(crate) fn match_rules(
        &self,
        candidates: &[usize],
        rules: &RuleSet,
        caller: &super::engine_types::Caller,
        request_body: &PermissionRequestBody,
    ) -> Option<PermissionResponse> {
        let (expanded_rules, expanded_indices) = self.expand_templates_sync(candidates, rules);

        let mut matching_rule_name: Option<String> = None;
        for &rule_idx in &expanded_indices {
            let rule = &expanded_rules[rule_idx];

            if !rule.subject.matches(caller) {
                continue;
            }
            if !self.rule_actions_match(rule, request_body) {
                continue;
            }

            matching_rule_name = Some(rule.name.clone());

            if rule.effect == Effect::Deny {
                let reason = format!("action denied by rule '{}'", rule.name);
                info!(
                    agent = %caller.agent,
                    result = "denied",
                    rule = %rule.name,
                    "permission check completed"
                );
                return Some(PermissionResponse::Denied {
                    reason,
                    rule: rule.name.clone(),
                    risk_level: assess_risk_level(request_body),
                });
            }
        }

        if matching_rule_name.is_some() {
            info!(
                agent = %caller.agent,
                result = "allowed",
                reason = "matched_rule",
                "permission check completed"
            );
            return Some(PermissionResponse::Allowed {
                token: generate_token(),
                context_modifier: None,
            });
        }
        None
    }
}

// --- Template expansion & utility helpers ---

impl PermissionEngine {
    /// Expand template references in candidate rules.
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
}

// --- Default & action matching ---

impl PermissionEngine {
    /// Get default response when no rule matches.
    fn default_response(
        &self,
        request: &PermissionRequestBody,
        defaults: &Defaults,
        reason: &str,
    ) -> PermissionResponse {
        // Step 1.8: ConfigWrite default Allow guard — design doc requires
        // "此维度永远高危，只能走单次审批". Even when defaults.config is Allow,
        // ConfigWrite must always be Denied via the default path.
        if matches!(request, PermissionRequestBody::ConfigWrite { .. }) {
            info!(
                agent = %request.agent_id(),
                result = "denied",
                reason = "config_write_default_guard",
                "permission check completed"
            );
            return PermissionResponse::Denied {
                reason: "config write is always high-risk, only single approval is allowed"
                    .to_string(),
                rule: "<config_write_default_guard>".to_string(),
                risk_level: assess_risk_level(request),
            };
        }

        let effect = match request {
            PermissionRequestBody::FileOp { op, .. } => match op.as_str() {
                "write" => defaults.file_write,
                _ => defaults.file_read,
            },
            PermissionRequestBody::CommandExec { .. } => defaults.command,
            PermissionRequestBody::NetOp { .. } => defaults.network,
            PermissionRequestBody::InterAgentMsg { .. } => defaults.inter_agent,
            PermissionRequestBody::ConfigWrite { .. } => defaults.config,
            PermissionRequestBody::SlashCommand { .. } => defaults.command,
            PermissionRequestBody::ToolCall { .. } => defaults.tool_call,
            PermissionRequestBody::MessageSend { .. } => defaults.message,
        };

        match effect {
            Effect::Allow => PermissionResponse::Allowed {
                token: generate_token(),
                context_modifier: None,
            },
            Effect::Deny => PermissionResponse::Denied {
                reason: reason.to_string(),
                rule: "default".to_string(),
                risk_level: assess_risk_level(request),
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

// --- Plan Mode helpers ---

/// Check if a file path belongs to the plans/ directory.
///
/// Returns `true` if path starts with `plans/` or contains `/plans/`.
fn is_plans_path(path: &str) -> bool {
    path.starts_with("plans/") || path.contains("/plans/")
}
