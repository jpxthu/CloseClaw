//! Mode-related slash command handlers.
//!
//! `/plan` enters Plan Mode; `/mode` queries or switches session mode.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::plan_state::{PlanPath, PlanStatus};
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::PlanPhase;
use closeclaw_common::PlanState;
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

        let plan_file_path = if let Some(conv) = self
            .session_manager
            .get_conversation_session(&ctx.session_id)
            .await
        {
            let cs = conv.read().await;
            let workdir = cs.workdir().to_path_buf();
            match plan_file::create_plan_file(&workdir, title) {
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

/// Parse `PlanStatus` from a plan file's content.
///
/// Scans the file for the `| 状态 | <status> |` line and converts it
/// to the corresponding `PlanStatus` variant.
pub(crate) fn parse_plan_status_from_file(content: &str) -> Option<PlanStatus> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("| 状态 | ") {
            let status_str = rest.strip_suffix(" |")?.trim();
            return match status_str {
                "draft" => Some(PlanStatus::Draft),
                "confirmed" => Some(PlanStatus::Confirmed),
                "executing" => Some(PlanStatus::Executing),
                "paused" => Some(PlanStatus::Paused),
                "completed" => Some(PlanStatus::Completed),
                _ => None,
            };
        }
    }
    None
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

// ── ExecuteHandler ────────────────────────────────────────────────────────

/// `/execute` — transition from Plan Mode to Auto Mode execution.
///
/// Checks that a plan has been approved (status `Confirmed` in the plan state)
/// before switching to Auto Mode. If the plan is not yet approved, replies
/// with a hint to use the approval tool first.
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

    async fn handle(&self, _args: &str, ctx: &SlashContext) -> SlashResult {
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

        // Step 3: Read the plan file and resolve status from file or in-memory state.
        //         Prefer the in-memory PlanStatus if already set (authoritative);
        //         fall back to parsing the plan file for backward compatibility.
        let file_status = match std::fs::read_to_string(&plan_state.plan_file_path) {
            Ok(content) => parse_plan_status_from_file(&content),
            Err(e) => {
                tracing::warn!(
                    plan_file = %plan_state.plan_file_path,
                    error = %e,
                    "Failed to read plan file for /execute"
                );
                return SlashResult::Reply(format!(
                    "无法读取 plan 文件：{}",
                    plan_state.plan_file_path
                ));
            }
        };

        // Use in-memory status if non-default; otherwise trust file-parsed status.
        let effective_status = if plan_state.status != PlanStatus::Draft {
            plan_state.status
        } else {
            match file_status {
                Some(s) => s,
                None => {
                    return SlashResult::Reply("Plan 文件中未找到有效的状态字段。".to_owned());
                }
            }
        };

        if effective_status != PlanStatus::Confirmed {
            return SlashResult::Reply(
                "Plan 尚未通过审批。请先使用 plan_approval 工具提交审批，".to_owned()
                    + "待 owner 审批通过后再执行 /execute。",
            );
        }

        // Step 4: Transition status draft→confirmed→executing and update plan file
        if let Err(e) = plan_state.transition_status(PlanStatus::Confirmed) {
            tracing::debug!(
                error = %e,
                "transition to Confirmed skipped (already confirmed)"
            );
        }
        if let Err(e) = plan_state.transition_status(PlanStatus::Executing) {
            tracing::warn!(
                error = %e,
                "Failed to transition plan status to Executing"
            );
            return SlashResult::Reply(format!("无法将 plan 状态转换为 executing：{}", e));
        }

        let path_clone = plan_state.plan_file_path.clone();
        let plan_file_path = std::path::PathBuf::from(&plan_state.plan_file_path);
        if let Err(e) = plan_file::update_plan_status(&path_clone, &PlanStatus::Executing) {
            tracing::warn!(
                plan_file = %path_clone,
                error = %e,
                "Failed to update plan file status to executing"
            );
        }

        // Persist updated plan state
        self.session_manager
            .set_plan_state(&ctx.session_id, plan_state)
            .await;

        // Plan is approved and executing — switch to Auto Mode
        SlashResult::SetMode {
            mode: "auto".to_owned(),
            plan_file_path: Some(plan_file_path),
        }
    }
}

// ── PauseHandler ─────────────────────────────────────────────────────────

/// `/pause` — pause an actively executing plan.
///
/// Switches the session from Auto Mode back to Plan Mode and updates
/// the plan status from `Executing` (or `Confirmed`) to `Paused`.
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
        let mut plan_state = match self.session_manager.get_plan_state(&ctx.session_id).await {
            Some(ps) => ps,
            None => {
                return SlashResult::Reply("当前没有活跃的 plan。".to_owned());
            }
        };

        if plan_state.plan_file_path.is_empty() {
            return SlashResult::Reply("当前 plan 没有关联的 plan 文件，无法暂停。".to_owned());
        }

        // Step 3: Transition status to Paused
        if let Err(e) = plan_state.transition_status(PlanStatus::Paused) {
            return SlashResult::Reply(format!("无法暂停 plan：{}", e));
        }

        // Step 4: Update plan file status to paused
        let path_clone = plan_state.plan_file_path.clone();
        if let Err(e) = plan_file::update_plan_status(&path_clone, &PlanStatus::Paused) {
            tracing::warn!(
                plan_file = %path_clone,
                error = %e,
                "Failed to update plan file status to paused"
            );
        }

        // Step 5: Persist updated plan state
        self.session_manager
            .set_plan_state(&ctx.session_id, plan_state)
            .await;

        // Step 6: Switch session mode back to Plan Mode
        SlashResult::SetMode {
            mode: "plan".to_owned(),
            plan_file_path: Some(std::path::PathBuf::from(&path_clone)),
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

        // Auto Mode is not a direct entry point — it can only be
        // reached through Plan Mode approval → /execute.
        if arg.eq_ignore_ascii_case("auto") {
            return SlashResult::Reply(
                "Auto Mode 不能直接通过 /mode auto 进入。".to_owned()
                    + "请先使用 /plan 进入 Plan Mode，"
                    + "完成规划并通过审批后使用 /execute 进入 Auto Mode。",
            );
        }

        // With argument — validate and return SetMode.
        let Some(target_mode) = SessionMode::from_str_opt(arg) else {
            return SlashResult::Reply(format!(
                "无效的会话模式：{arg}。可选值：normal, plan, auto"
            ));
        };

        // Approval gate: `/mode normal` from Plan Mode is forbidden.
        if target_mode == SessionMode::Normal {
            let Some(conv) = self
                .session_manager
                .get_conversation_session(&ctx.session_id)
                .await
            else {
                return SlashResult::Reply("当前会话未激活".to_owned());
            };
            let cs = conv.read().await;
            if cs.session_mode() == SessionMode::Plan {
                return SlashResult::Reply(
                    "Plan Mode 下不能直接切换到 Normal Mode。".to_owned()
                        + "请使用 plan_approval 工具提交审批，"
                        + "审批通过后方可退出 Plan Mode。",
                );
            }
        }

        SlashResult::SetMode {
            mode: target_mode.to_string(),
            plan_file_path: None,
        }
    }
}
