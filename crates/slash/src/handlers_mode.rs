//! Mode-related slash command handlers.
//!
//! `/plan` enters Plan Mode; `/mode` queries or switches session mode.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::plan_state::PlanPath;
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::{ModeTransition, PlanPhase, PlanState, SessionLookup};
use closeclaw_gateway::SessionManager;
use closeclaw_session::plan_file;
use tracing;

// ── PlanModeHandler ───────────────────────────────────────────────────────

/// `/plan` — enter Plan Mode with an optional task description.
///
/// - With arguments: creates a plan file in the session's workdir,
///   returns `SlashResult::SetMode` with the plan file path.
/// - Without arguments: replies with a usage hint.
pub struct PlanModeHandler {
    session_manager: Arc<SessionManager>,
}

impl PlanModeHandler {
    /// Create a new PlanModeHandler with access to session state.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for PlanModeHandler {
    fn commands(&self) -> &[&str] {
        &["plan"]
    }

    fn description(&self) -> &str {
        "进入 Plan Mode"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        if args.trim().is_empty() {
            return SlashResult::Reply(
                "用法：/plan [--path standard|interview] <任务描述>\n进入 Plan Mode 进行任务规划。"
                    .to_owned(),
            );
        }

        // Parse --path argument and extract task title
        let (explicit_path, title) = parse_plan_path_arg(args.trim());
        if title.trim().is_empty() {
            return SlashResult::Reply(
                "用法：/plan [--path standard|interview] <任务描述>\n进入 Plan Mode 进行任务规划。"
                    .to_owned(),
            );
        }

        // Read conversation session once to get both mode and workdir,
        // avoiding a second async read lock acquisition.
        let (exiting_auto, workdir) = if let Some(conv) = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        {
            let cs = conv.read().await;
            let exiting_auto = cs.session_mode() == SessionMode::Auto;
            let workdir = cs.workdir().to_path_buf();
            (exiting_auto, Some(workdir))
        } else {
            (false, None)
        };

        // Inject mode transition based on prior state.
        // - Auto Mode exit → ExitAuto (priority: leaving Auto is more important
        //   than reentry notification)
        // - Plan Mode re-entry (from Normal or other) → Reentry
        if exiting_auto {
            // Exiting Auto Mode via /plan — inject ExitAuto transition.
            self.session_manager
                .set_pending_mode_transition(&ctx.session_id, ModeTransition::ExitAuto)
                .await;
        } else if let Some(prev_plan_state) =
            self.session_manager.get_plan_state(&ctx.session_id).await
        {
            if !prev_plan_state.plan_file_path.is_empty() {
                self.session_manager
                    .set_pending_mode_transition(&ctx.session_id, ModeTransition::Reentry)
                    .await;
            }
        }

        let plan_file_path = if let Some(ref workdir) = workdir {
            match plan_file::create_plan_file(workdir, title) {
                Ok(path) => Some(path),
                Err(e) => {
                    tracing::warn!(
                        title = %title,
                        error = %e,
                        "Failed to create plan file, proceeding without it"
                    );
                    None
                }
            }
        } else {
            None
        };

        // Initialize PlanState with the plan file path and explicit path
        if let Some(ref path) = plan_file_path {
            let mut plan_state = PlanState::new();
            plan_state.plan_file_path = path.to_string_lossy().to_string();
            plan_state.phase = PlanPhase::Research;
            plan_state.explicit_path = explicit_path;
            self.session_manager
                .set_plan_state(&ctx.session_id, plan_state)
                .await;
        }

        SlashResult::SetMode {
            mode: "plan".to_owned(),
            plan_file_path,
        }
    }
}

/// Parse `--path` argument from the `/plan` command.
///
/// Returns `(Some(PlanPath), remaining_title)` when `--path standard` or
/// `--path interview` is found; `(None, original_args)` otherwise.
/// The task title is the remaining args after stripping `--path <value>`.
pub(crate) fn parse_plan_path_arg(args: &str) -> (Option<PlanPath>, &str) {
    let trimmed = args.trim();
    if let Some(rest) = trimmed.strip_prefix("--path") {
        let rest = rest.trim_start();
        if let Some(value_end) = rest.find(|c: char| c.is_whitespace()) {
            let value = &rest[..value_end];
            let title = rest[value_end..].trim();
            let path = match value {
                "standard" => Some(PlanPath::Standard),
                "interview" => Some(PlanPath::Interview),
                _ => {
                    tracing::warn!(
                        path_value = %value,
                        "Invalid --path value, ignoring"
                    );
                    None
                }
            };
            (path, title)
        } else if rest.is_empty() {
            // --path with nothing after it
            (None, trimmed)
        } else if matches!(rest, "standard" | "interview") {
            // --path with a recognized value but no title following
            let path = match rest {
                "standard" => Some(PlanPath::Standard),
                _ => Some(PlanPath::Interview),
            };
            (path, "")
        } else {
            // --path with unrecognized value (no title) — treat as invalid path, rest is title
            (None, rest)
        }
    } else {
        (None, trimmed)
    }
}

/// Parse optional step selection from `/execute` args.
///
/// Accepts comma-separated 0-based step indices (e.g., `"0,1,2"`) or
/// an empty string (returns `None` for all steps). Returns `None` if
/// the args are empty or contain only whitespace.
pub(crate) fn parse_step_selection_arg(args: &str) -> Option<Vec<usize>> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return None;
    }
    let indices: Vec<usize> = trimmed
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<usize>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if indices.is_empty() {
        None
    } else {
        Some(indices)
    }
}

// ── AutoModeHandler ──────────────────────────────────────────────────────

/// `/auto` — directly enter Auto Mode.
///
/// - Without arguments: switches to Auto Mode without a plan file.
/// - With a plan file path: validates the file exists, initializes
///   `PlanState`, and switches to Auto Mode.
/// - If already in Auto Mode: replies with a notification.
/// - If in Plan Mode: injects `ExitPlan` transition before switching.
pub struct AutoModeHandler {
    session_manager: Arc<SessionManager>,
}

impl AutoModeHandler {
    /// Create a new AutoModeHandler with access to session state.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for AutoModeHandler {
    fn commands(&self) -> &[&str] {
        &["auto"]
    }

    fn description(&self) -> &str {
        "直接进入 Auto Mode"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let Some(conv) = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        else {
            return SlashResult::Reply("当前会话未激活".to_owned());
        };

        // Read current mode and workdir in a single lock acquisition.
        let (current_mode, workdir) = {
            let cs = conv.read().await;
            (cs.session_mode(), cs.workdir().to_path_buf())
        };

        if current_mode == SessionMode::Auto {
            return SlashResult::Reply("已在 Auto Mode".to_owned());
        }

        let plan_arg = args.trim();
        let plan_file_path = if plan_arg.is_empty() {
            None
        } else {
            let path = std::path::PathBuf::from(plan_arg);
            if !path.exists() {
                return SlashResult::Reply(format!("plan 文件不存在：{}", path.display()));
            }
            Some(path)
        };

        // Initialize PlanState when a plan file is provided.
        if let Some(ref path) = plan_file_path {
            let mut plan_state = PlanState::new();
            plan_state.plan_file_path = path.to_string_lossy().to_string();
            plan_state.phase = PlanPhase::FinalPlan;
            self.session_manager
                .set_plan_state(&ctx.session_id, plan_state)
                .await;
        }

        // Inject ExitPlan transition when leaving Plan Mode.
        if current_mode == SessionMode::Plan {
            self.session_manager
                .set_pending_mode_transition(&ctx.session_id, ModeTransition::ExitPlan)
                .await;
        }

        let plan_file_path_for_result = plan_file_path.map(|p| {
            // Ensure the path is absolute for persistence.
            if p.is_absolute() {
                p
            } else {
                workdir.join(p)
            }
        });

        SlashResult::SetMode {
            mode: "auto".to_owned(),
            plan_file_path: plan_file_path_for_result,
        }
    }
}

// ── ExecuteHandler ────────────────────────────────────────────────────────

/// `/execute` — transition from Plan Mode to Auto Mode execution.
///
/// Validates that a plan file exists, then switches to Auto Mode.
pub struct ExecuteHandler {
    session_manager: Arc<SessionManager>,
}

impl ExecuteHandler {
    /// Create a new ExecuteHandler with access to session state.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for ExecuteHandler {
    fn commands(&self) -> &[&str] {
        &["execute"]
    }

    fn description(&self) -> &str {
        "从 Plan Mode 进入 Auto Mode 执行"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        // Parse optional step_selection from args (e.g., "/execute 0,1,2")
        let step_selection = parse_step_selection_arg(args.trim());

        // Step 1: Check session is in Plan Mode
        let Some(conv) = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        else {
            return SlashResult::Reply("当前会话未激活".to_owned());
        };
        {
            let cs = conv.read().await;
            if cs.session_mode() != SessionMode::Plan {
                return SlashResult::Reply(
                    "/execute 需要在 Plan Mode 下使用。先用 /plan <任务描述> 进入 Plan Mode。"
                        .to_owned(),
                );
            }
        }

        // Step 2: Load plan_state from checkpoint
        let mut plan_state = match self.session_manager.get_plan_state(&ctx.session_id).await {
            Some(ps) => ps,
            None => {
                return SlashResult::Reply(
                    "当前没有活跃的 plan。请先用 /plan <任务描述> 创建一个 plan。".to_owned(),
                );
            }
        };

        if plan_state.plan_file_path.is_empty() {
            return SlashResult::Reply("当前 plan 没有关联的 plan 文件，无法执行。".to_owned());
        }

        let plan_file_path = std::path::PathBuf::from(&plan_state.plan_file_path);

        // Store step_selection in plan_state for the execution engine.
        plan_state.step_selection = step_selection;

        // Persist updated plan state
        self.session_manager
            .set_plan_state(&ctx.session_id, plan_state)
            .await;

        // Plan exists — inject ExitPlan transition notification before switching to Auto Mode.
        self.session_manager
            .set_pending_mode_transition(&ctx.session_id, ModeTransition::ExitPlan)
            .await;

        SlashResult::SetMode {
            mode: "auto".to_owned(),
            plan_file_path: Some(plan_file_path),
        }
    }
}

// ── PauseHandler ─────────────────────────────────────────────────────────

/// `/pause` — pause an actively executing plan.
///
/// Switches the session from Auto Mode back to Plan Mode.
pub struct PauseHandler {
    session_manager: Arc<SessionManager>,
}

impl PauseHandler {
    /// Create a new PauseHandler with access to session state.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for PauseHandler {
    fn commands(&self) -> &[&str] {
        &["pause"]
    }

    fn description(&self) -> &str {
        "暂停正在执行的 plan"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, _args: &str, ctx: &SlashContext) -> SlashResult {
        // Step 1: Check session is in Auto Mode
        let Some(conv) = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        else {
            return SlashResult::Reply("当前会话未激活".to_owned());
        };
        {
            let cs = conv.read().await;
            if cs.session_mode() != SessionMode::Auto {
                return SlashResult::Reply(
                    "/pause 需要在 Auto Mode 下使用。当前没有正在执行的 plan。".to_owned(),
                );
            }
        }

        // Step 2: Load plan state
        let plan_state = match self.session_manager.get_plan_state(&ctx.session_id).await {
            Some(ps) => ps,
            None => {
                return SlashResult::Reply("当前没有活跃的 plan。".to_owned());
            }
        };

        if plan_state.plan_file_path.is_empty() {
            return SlashResult::Reply("当前 plan 没有关联的 plan 文件，无法暂停。".to_owned());
        }

        // Step 3: Persist plan state as-is (no status transition needed)
        let plan_file_path = std::path::PathBuf::from(&plan_state.plan_file_path);
        self.session_manager
            .set_plan_state(&ctx.session_id, plan_state)
            .await;

        // Inject Reentry transition (re-entering Plan Mode).
        self.session_manager
            .set_pending_mode_transition(&ctx.session_id, ModeTransition::Reentry)
            .await;

        // Step 4: Switch session mode back to Plan Mode
        SlashResult::SetMode {
            mode: "plan".to_owned(),
            plan_file_path: Some(plan_file_path),
        }
    }
}

// ── ModeHandler ──────────────────────────────────────────────────────────

/// `/mode` — query or switch the session mode.
///
/// - No arguments: reads the current `SessionMode` and replies.
/// - With an argument (`normal`, `plan`, `auto`): returns
///   `SlashResult::SetMode` to trigger the mode switch.
pub struct ModeHandler {
    session_manager: Arc<SessionManager>,
}

impl ModeHandler {
    /// Create a new ModeHandler operating on the given session manager.
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for ModeHandler {
    fn commands(&self) -> &[&str] {
        &["mode"]
    }

    fn description(&self) -> &str {
        "查询或切换会话模式"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        true
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let arg = args.trim();

        // No arguments — return the current session mode.
        if arg.is_empty() {
            let Some(conv) = self
                .session_manager
                .get_conversation_session(&ctx.session_id)
                .await
            else {
                return SlashResult::Reply("当前会话未激活".to_owned());
            };
            let cs = conv.read().await;
            let mode = cs.session_mode();
            return SlashResult::Reply(format!("当前会话模式：{mode}"));
        }

        // Auto Mode should be entered via the dedicated /auto command.
        if arg.eq_ignore_ascii_case("auto") {
            return SlashResult::Reply("请使用 /auto 命令直接进入 Auto Mode。".to_owned());
        }

        // With argument — validate and return SetMode.
        let Some(target_mode) = SessionMode::from_str_opt(arg) else {
            return SlashResult::Reply(format!(
                "无效的会话模式：{arg}。可选值：normal, plan, auto"
            ));
        };

        // Read current mode for approval gate and ExitAuto detection.
        let current_mode = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await;
        let current_mode = if let Some(conv) = current_mode {
            Some(conv.read().await.session_mode())
        } else {
            None
        };

        // Approval gate: `/mode normal` from Plan Mode is forbidden.
        if target_mode == SessionMode::Normal && current_mode == Some(SessionMode::Plan) {
            return SlashResult::Reply(
                "Plan Mode 下不能直接切换到 Normal Mode。".to_owned()
                    + "请使用 plan_approval 工具提交审批，"
                    + "审批通过后方可退出 Plan Mode。",
            );
        }

        // Inject ExitAuto transition when leaving Auto Mode.
        if current_mode == Some(SessionMode::Auto) && target_mode != SessionMode::Auto {
            self.session_manager
                .set_pending_mode_transition(&ctx.session_id, ModeTransition::ExitAuto)
                .await;
        }

        SlashResult::SetMode {
            mode: target_mode.to_string(),
            plan_file_path: None,
        }
    }
}
