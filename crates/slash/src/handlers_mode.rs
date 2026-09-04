//! Mode-related slash command handlers.
//!
//! `/plan` enters Plan Mode; `/mode` queries or switches session mode.

use std::path::Path;
use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::SlashSessionQuery;
use closeclaw_common::{PlanPhase, PlanState};
use closeclaw_config::IdentifierFormat;
use closeclaw_session::plan_file;
use tracing;

// ── PlanModeHandler ───────────────────────────────────────────────────────

/// `/plan` — enter Plan Mode with an optional task description.
///
/// - With arguments: creates a plan file in the session's workdir,
///   returns `SlashResult::SetMode` with the plan file path.
/// - Without arguments: enters Plan Mode without creating a plan file.
#[derive(Clone)]
pub struct PlanModeHandler {
    session_manager: Arc<dyn SlashSessionQuery>,
    identifier_format: IdentifierFormat,
}

impl PlanModeHandler {
    /// Create a new PlanModeHandler with access to session state.
    pub fn new(
        session_manager: Arc<dyn SlashSessionQuery>,
        identifier_format: IdentifierFormat,
    ) -> Self {
        Self {
            session_manager,
            identifier_format,
        }
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

    fn immediate(&self, _cmd: &str, _args: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let title = args.trim();
        let has_title = !title.is_empty();

        // Read workdir via trait method.
        let workdir = self.session_manager.get_workdir(&ctx.session_id).await;

        // Mode transition injection removed (design doc §6 — transition prompts
        // are no longer injected via System Prompt sections).

        // No title: enter Plan Mode without creating a plan file.
        if !has_title {
            return SlashResult::SetMode {
                mode: "plan".to_owned(),
                plan_file_path: None,
                initial_input: None,
                reply_message: Some("已切换到 Plan 模式".to_owned()),
            };
        }

        let plan_file_path = if let Some(ref workdir) = workdir {
            match plan_file::create_plan_file_with_format(workdir, title, self.identifier_format) {
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
            self.session_manager
                .set_plan_state(&ctx.session_id, plan_state)
                .await;
        }

        SlashResult::SetMode {
            mode: "plan".to_owned(),
            plan_file_path,
            initial_input: has_title.then(|| title.to_owned()),
            reply_message: Some("已切换到 Plan 模式".to_owned()),
        }
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }
}

/// Parse `/execute` arguments into plan name and additional instruction.
///
/// The first whitespace-delimited token is treated as the plan name;
/// everything after the first space is the additional instruction.
///
/// # Examples
///
/// - `"foo bar baz"` → `("foo", Some("bar baz"))`
/// - `"foo"` → `("foo", None)`
/// - `""` → `("", None)`
///
/// The instruction preserves all whitespace after the first space,
/// matching the doc spec: "空格后的内容".
pub(crate) fn parse_execute_args(args: &str) -> (String, Option<String>) {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }
    match trimmed.split_once(char::is_whitespace) {
        Some((name, rest)) => {
            let instruction = rest.trim();
            (
                name.to_owned(),
                if instruction.is_empty() {
                    None
                } else {
                    Some(instruction.to_owned())
                },
            )
        }
        None => (trimmed.to_owned(), None),
    }
}

/// Resolve a relative plan file path to absolute and refresh its
/// application-layer access timestamp.
///
/// Failures are logged as warnings but do not propagate — the caller
/// should continue regardless of whether the touch succeeded.
fn refresh_plan_access_timestamp(plan_file_path: Option<&Path>, workdir: Option<&Path>) {
    let Some(path) = plan_file_path else {
        return;
    };
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workdir
            .map(|wd| wd.join(path))
            .unwrap_or_else(|| path.to_path_buf())
    };
    if let Err(e) = plan_file::touch_access_timestamp(&abs_path) {
        tracing::warn!(
            plan_file = %abs_path.display(),
            error = %e,
            "failed to touch access timestamp in /execute"
        );
    }
}

// ── ExecuteHandler ────────────────────────────────────────────────────────

/// `/execute <plan名称> [附加指令]` — transition to Auto Mode execution.
///
/// The plan name is **required**; calling without a plan name returns
/// a usage hint.
///
/// - In Plan Mode: resolves the plan by name, then switches to Auto Mode.
/// - In non-Plan Mode: resolves the plan by name and switches to Auto Mode.
/// - If additional instructions are provided after the plan name, they
///   are injected as the initial user message via `initial_input`.
#[derive(Clone)]
pub struct ExecuteHandler {
    session_manager: Arc<dyn SlashSessionQuery>,
}

impl ExecuteHandler {
    /// Create a new ExecuteHandler with access to session state.
    pub fn new(session_manager: Arc<dyn SlashSessionQuery>) -> Self {
        Self { session_manager }
    }

    /// Non-Plan Mode: resolve plan by name, then enter Auto Mode.
    fn handle_non_plan_mode(
        &self,
        plan_name: &str,
        workdir: Option<&std::path::Path>,
        instruction: Option<&str>,
    ) -> SlashResult {
        if plan_name.is_empty() {
            return SlashResult::Reply(
                "请指定要执行的 plan 名称。用法：/execute <plan名称> [附加指令]".to_owned(),
            );
        }

        let plan_file_path = match workdir {
            Some(wd) => match plan_file::resolve_plan_by_name(wd, plan_name) {
                Ok(path) => Some(path),
                Err(e) => {
                    return SlashResult::Reply(format!("计划文件解析失败：{e}"));
                }
            },
            None => {
                return SlashResult::Reply("没有工作目录，无法按名称定位 plan。".to_owned());
            }
        };
        // Refresh access timestamp so the plan does not get archived prematurely
        refresh_plan_access_timestamp(plan_file_path.as_deref(), workdir);

        SlashResult::SetMode {
            mode: "auto".to_owned(),
            plan_file_path,
            initial_input: instruction.map(String::from),
            reply_message: Some("开始执行".to_owned()),
        }
    }

    /// Plan Mode: resolve plan by name, then enter Auto Mode.
    async fn handle_plan_mode(
        &self,
        _ctx: &SlashContext,
        plan_name: &str,
        workdir: Option<&std::path::Path>,
        instruction: Option<&str>,
    ) -> SlashResult {
        if plan_name.is_empty() {
            return SlashResult::Reply(
                "请指定要执行的 plan 名称。用法：/execute <plan名称> [附加指令]".to_owned(),
            );
        }

        let plan_file_path = match workdir {
            Some(wd) => match plan_file::resolve_plan_by_name(wd, plan_name) {
                Ok(path) => path,
                Err(e) => {
                    return SlashResult::Reply(format!("计划文件解析失败：{e}"));
                }
            },
            None => {
                return SlashResult::Reply("没有工作目录，无法按名称定位 plan。".to_owned());
            }
        };
        // Refresh access timestamp so the plan does not get archived prematurely
        refresh_plan_access_timestamp(Some(&plan_file_path), workdir);

        SlashResult::SetMode {
            mode: "auto".to_owned(),
            plan_file_path: Some(plan_file_path),
            initial_input: instruction.map(String::from),
            reply_message: Some("开始执行".to_owned()),
        }
    }
}

#[async_trait::async_trait]
impl SlashHandler for ExecuteHandler {
    fn commands(&self) -> &[&str] {
        &["execute"]
    }

    fn description(&self) -> &str {
        "/execute <plan名称> [附加指令] — 进入 Auto Mode 执行 plan"
    }

    fn immediate(&self, _cmd: &str, _args: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let mode = match self.session_manager.get_session_mode(&ctx.session_id).await {
            Some(m) => m,
            None => return SlashResult::Reply("当前会话未激活".to_owned()),
        };

        let (plan_name, instruction) = parse_execute_args(args);
        let workdir = self.session_manager.get_workdir(&ctx.session_id).await;
        let workdir_ref = workdir.as_deref();

        if mode != SessionMode::Plan {
            return self.handle_non_plan_mode(&plan_name, workdir_ref, instruction.as_deref());
        }

        self.handle_plan_mode(ctx, &plan_name, workdir_ref, instruction.as_deref())
            .await
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }
}

// ── ModeHandler ──────────────────────────────────────────────────────────

/// `/mode` — query or switch the session mode.
///
/// - No arguments: reads the current `SessionMode` and replies.
/// - With an argument (`normal`, `plan`): returns
///   `SlashResult::SetMode` to trigger the mode switch.
/// - `auto` is not a valid `/mode` target (use `/execute` instead).
#[derive(Clone)]
pub struct ModeHandler {
    session_manager: Arc<dyn SlashSessionQuery>,
    plan_handler: Option<Arc<PlanModeHandler>>,
}

impl ModeHandler {
    /// Create a new ModeHandler operating on the given session manager.
    pub fn new(session_manager: Arc<dyn SlashSessionQuery>) -> Self {
        Self {
            session_manager,
            plan_handler: None,
        }
    }

    /// Create a ModeHandler with a delegated plan handler.
    ///
    /// `/mode plan` is delegated to `PlanModeHandler` so it produces
    /// the same side effects as `/plan`.
    pub fn with_handlers(
        session_manager: Arc<dyn SlashSessionQuery>,
        plan_handler: Arc<PlanModeHandler>,
    ) -> Self {
        Self {
            session_manager,
            plan_handler: Some(plan_handler),
        }
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

    fn immediate(&self, _cmd: &str, args: &str) -> bool {
        args.trim().is_empty()
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let arg = args.trim();

        // No arguments — return the current session mode.
        if arg.is_empty() {
            let mode = match self.session_manager.get_session_mode(&ctx.session_id).await {
                Some(m) => m,
                None => return SlashResult::Reply("当前会话未激活".to_owned()),
            };
            let display = match mode {
                SessionMode::Normal => "Normal",
                SessionMode::Plan => "Plan",
                SessionMode::Auto => "Auto",
            };
            return SlashResult::Reply(format!("当前模式：{display}"));
        }

        // Split mode name from remaining args: "plan 任务描述" → ("plan", "任务描述")
        let (mode_str, remaining_args) = match arg.split_once(char::is_whitespace) {
            Some((m, rest)) => (m, rest),
            None => (arg, ""),
        };

        let Some(target_mode) = SessionMode::from_str_opt(mode_str) else {
            return SlashResult::Reply("无效模式。可用：normal, plan".to_owned());
        };

        // `auto` is parsed by from_str_opt but is not a valid /mode target.
        // Only normal and plan are allowed; auto must go through /execute.
        if target_mode == SessionMode::Auto {
            return SlashResult::Reply("无效模式。可用：normal, plan".to_owned());
        }

        // Delegate /mode plan to PlanModeHandler
        // so the behavior is equivalent to /plan.
        if target_mode == SessionMode::Plan {
            if let Some(ref plan_handler) = self.plan_handler {
                return plan_handler.handle(remaining_args, ctx).await;
            }
        }

        SlashResult::SetMode {
            mode: target_mode.to_string(),
            plan_file_path: None,
            initial_input: None,
            reply_message: Some("已切换到 Normal 模式".to_owned()),
        }
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }
}
