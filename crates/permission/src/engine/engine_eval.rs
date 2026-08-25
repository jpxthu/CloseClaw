//! Permission Engine - Evaluation logic.

use super::audit_log::{build_audit_log, AuditDisposition, AuditLogger};
use super::engine_helpers::{generate_token, get_agent_deny_subjects, resolve_template_actions};
use super::engine_matching::action_matches_request;
use super::engine_risk::{assess_risk_level, RiskLevel};
use super::engine_types::{
    Caller, Defaults, Effect, PermissionRequest, PermissionRequestBody, PermissionResponse, Rule,
    RuleSet, Subject,
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

/// Callback type for submitting auto-mode dangerous operations to the
/// approval flow.
///
/// # Arguments
/// * `caller` — Who initiated the operation.
/// * `body` — The permission request body.
/// * `risk_level` — Risk assessment of the operation.
/// * `agent_id` — The agent instance ID.
///
/// # Returns
/// `Some(request_id)` if the operation was enqueued for owner approval,
/// `None` if the submission was rejected (e.g., sub-agent or duplicate).
pub type ApprovalCallback =
    Arc<dyn Fn(&Caller, &PermissionRequestBody, RiskLevel, &str) -> Option<String> + Send + Sync>;

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
            Subject::UserOnly { user_id, .. } => {
                // UserOnly: index by user_id only; agent is wildcard "*".
                let key = format!("{}:*", user_id);
                user_agent_index.entry(key).or_default().push(idx);
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
    /// Optional audit logger for recording both approved and rejected
    /// permission requests in Auto Mode.
    audit_logger: Option<Arc<dyn AuditLogger>>,
    /// Optional callback for submitting auto-mode dangerous operations
    /// to the approval flow. When set, high/critical risk operations
    /// in Auto Mode are routed through this callback instead of being
    /// directly denied.
    approval_callback: Option<ApprovalCallback>,
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
            audit_logger: None,
            approval_callback: None,
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

    /// Inject an audit logger for recording approved and rejected permission requests.
    pub fn with_audit_logger(mut self, logger: Arc<dyn AuditLogger>) -> Self {
        self.audit_logger = Some(logger);
        self
    }

    /// Get a reference to the audit logger, if set.
    pub fn audit_logger(&self) -> Option<&Arc<dyn AuditLogger>> {
        self.audit_logger.as_ref()
    }

    /// Inject an approval callback for auto-mode dangerous operations.
    ///
    /// When set, high/critical risk operations in Auto Mode are routed
    /// through this callback instead of being directly denied. The
    /// callback receives the caller, request body, risk level, and
    /// agent ID, and returns an optional approval request ID.
    pub fn with_approval_callback(mut self, callback: ApprovalCallback) -> Self {
        self.approval_callback = Some(callback);
        self
    }

    /// Submit an auto-mode dangerous operation to the approval flow.
    ///
    /// Returns `Some(request_id)` if the operation was enqueued for
    /// owner approval, or `None` if the approval flow rejected the
    /// submission (e.g., sub-agent or duplicate).
    pub(super) fn submit_auto_mode_approval(
        &self,
        caller: &Caller,
        body: &PermissionRequestBody,
        risk_level: RiskLevel,
        agent_id: &str,
    ) -> Option<String> {
        let callback = self.approval_callback.as_ref()?;
        callback(caller, body, risk_level, agent_id)
    }

    /// Log a rejection if the logger is set, the response is `Denied`,
    /// and the session is in Auto Mode.
    ///
    /// Per design doc: rejection logs are only generated for Auto Mode
    /// sessions (Plan/Normal/unknown modes do not produce logs).
    ///
    /// When an audit logger is also configured, the rejection is additionally
    /// recorded in the audit log with `AuditDisposition::Rejected`.
    fn log_rejection(&self, response: &PermissionResponse, body: &PermissionRequestBody) {
        if let PermissionResponse::Denied {
            reason, risk_level, ..
        } = response
        {
            // Determine session mode from query (best-effort).
            let session_mode = self
                .session_mode_query
                .as_ref()
                .and_then(|q| q.get_session_mode(body.agent_id()));

            // Record rejection log (Auto Mode only).
            if let Some(logger) = &self.rejection_logger {
                if session_mode == Some(SessionMode::Auto) {
                    let entry =
                        build_rejection_log(body, reason.clone(), *risk_level, session_mode);
                    logger.log(&entry);
                }
            }

            // Also record in audit log (Auto Mode only).
            if session_mode == Some(SessionMode::Auto) {
                self.log_audit(
                    AuditDisposition::Rejected,
                    body,
                    reason.clone(),
                    *risk_level,
                    session_mode,
                );
            }
        }
    }

    /// Record an entry in the audit log with the given disposition.
    ///
    /// Only writes when an audit logger is configured and the session
    /// is in Auto Mode.
    fn log_audit(
        &self,
        disposition: AuditDisposition,
        body: &PermissionRequestBody,
        reason: String,
        risk_level: RiskLevel,
        session_mode: Option<SessionMode>,
    ) {
        if let Some(logger) = &self.audit_logger {
            let entry = build_audit_log(body, disposition, reason, risk_level, session_mode);
            logger.log(&entry);
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

        // Step 0.1: Auto Mode runtime dangerous-operation review
        let is_owner = caller.user_id == "owner";
        if !is_owner {
            if let Some(denied) = self.check_auto_mode_filter(&request, &agent_id) {
                self.log_rejection(&denied, request.body());
                return denied;
            }
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
                    approval_request_id: None,
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
                    approval_request_id: None,
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

        // Step 2: User phase — collect UserAndAgent + UserOnly candidates and evaluate
        let user_candidates = self.collect_user_agent_candidates_with_index(
            &caller,
            &agent_id,
            rules,
            user_agent_rule_index,
        );
        let (user_result, user_only_matched) =
            self.match_rules_with_info(&user_candidates, rules, &caller, request.body());

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
                    approval_request_id: None,
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
                approval_request_id: None,
            },
            (_, Some(PermissionResponse::Denied { .. })) => PermissionResponse::Denied {
                reason: "action denied by user rule".to_string(),
                rule: "<user_phase>".to_string(),
                risk_level: assess_risk_level(request.body()),
                approval_request_id: None,
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
            // No agent rule, but UserOnly rule allowed → user-only allow is sufficient
            // (UserAndAgent Allow without Agent Allow falls through to defaults → Denied)
            (None, Some(PermissionResponse::Allowed { .. })) if user_only_matched => {
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
                        approval_request_id: None,
                    };
                    self.log_rejection(&extra_denied, request.body());
                    return extra_denied;
                }
            }
        }

        response
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

        // Exact match: "{user_id}:{agent_id}" (UserAndAgent rules)
        let index_key = format!("{}:{}", caller.user_id, agent_id);
        if let Some(indices) = user_agent_rule_index.get(&index_key) {
            candidates.extend(indices);
        }

        // UserOnly match: "{user_id}:*" (matches any agent)
        let user_only_key = format!("{}:*", caller.user_id);
        if let Some(indices) = user_agent_rule_index.get(&user_only_key) {
            candidates.extend(indices);
        }

        if candidates.is_empty() {
            for (idx, rule) in rules.rules.iter().enumerate() {
                if (rule.subject.is_user_and_agent() || rule.subject.is_user_only())
                    && rule.subject.matches(caller)
                {
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
        let (result, _user_only) =
            self.match_rules_with_info(candidates, rules, caller, request_body);
        result
    }

    /// Like [`match_rules`] but also returns whether the Allow came from a
    /// UserOnly rule (needed for two-phase merge: UserOnly Allow alone is
    /// sufficient, whereas UserAndAgent Allow requires Agent phase agreement).
    pub(crate) fn match_rules_with_info(
        &self,
        candidates: &[usize],
        rules: &RuleSet,
        caller: &super::engine_types::Caller,
        request_body: &PermissionRequestBody,
    ) -> (Option<PermissionResponse>, bool) {
        let (expanded_rules, expanded_indices) = self.expand_templates_sync(candidates, rules);

        let mut matching_rule_name: Option<String> = None;
        let mut user_only_matched = false;
        for &rule_idx in &expanded_indices {
            let rule = &expanded_rules[rule_idx];

            if !rule.subject.matches(caller) {
                continue;
            }
            if !self.rule_actions_match(rule, request_body) {
                continue;
            }

            matching_rule_name = Some(rule.name.clone());
            if rule.subject.is_user_only() {
                user_only_matched = true;
            }

            if rule.effect == Effect::Deny {
                let reason = format!("action denied by rule '{}'", rule.name);
                info!(
                    agent = %caller.agent,
                    result = "denied",
                    rule = %rule.name,
                    "permission check completed"
                );
                return (
                    Some(PermissionResponse::Denied {
                        reason,
                        rule: rule.name.clone(),
                        risk_level: assess_risk_level(request_body),
                        approval_request_id: None,
                    }),
                    user_only_matched,
                );
            }
        }

        if matching_rule_name.is_some() {
            info!(
                agent = %caller.agent,
                result = "allowed",
                reason = "matched_rule",
                "permission check completed"
            );
            return (
                Some(PermissionResponse::Allowed {
                    token: generate_token(),
                    context_modifier: None,
                }),
                user_only_matched,
            );
        }
        (None, false)
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
                approval_request_id: None,
            };
        }

        let effect = match request {
            PermissionRequestBody::FileOp { op, .. } => match op.as_str() {
                "write" => defaults.file_write,
                _ => defaults.file_read,
            },
            PermissionRequestBody::CommandExec { .. } => defaults.exec,
            PermissionRequestBody::NetOp { .. } => defaults.network,
            PermissionRequestBody::InterAgentMsg { .. } => defaults.inter_agent,
            PermissionRequestBody::ConfigWrite { .. } => defaults.config,
            PermissionRequestBody::SlashCommand { .. } => defaults.exec,
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
                approval_request_id: None,
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
