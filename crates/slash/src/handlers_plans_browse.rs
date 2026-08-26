//! `/plans` slash handler — list or view plans in the workspace.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::SlashSessionQuery;
use closeclaw_session::plan_file;

/// `/plans` — list or view plans in the workspace.
///
/// - `/plans` (no arguments): list all plans in the session's workdir
///   with title and task completion summary, sorted by modification
///   time (most recent first).
/// - `/plans <name>`: show the full content of the matching plan.
///   Uses the three-tier resolution strategy (exact → prefix → fuzzy).
#[derive(Clone)]
pub struct PlanBrowseHandler {
    session_manager: Arc<dyn SlashSessionQuery>,
}

impl PlanBrowseHandler {
    /// Create a new PlanBrowseHandler.
    pub fn new(session_manager: Arc<dyn SlashSessionQuery>) -> Self {
        Self { session_manager }
    }
}

#[async_trait::async_trait]
impl SlashHandler for PlanBrowseHandler {
    fn commands(&self) -> &[&str] {
        &["plans"]
    }

    fn description(&self) -> &str {
        "列出或查看工作区中的 plan"
    }

    fn immediate(&self, _cmd: &str) -> bool {
        false
    }

    async fn handle(&self, args: &str, ctx: &SlashContext) -> SlashResult {
        let workdir = match self.session_manager.get_workdir(&ctx.session_id).await {
            Some(wd) => wd,
            None => {
                return SlashResult::Reply("当前会话未设置工作目录。".to_owned());
            }
        };

        let name = args.trim();
        if name.is_empty() {
            self.list_plans(&workdir)
        } else {
            self.view_plan(&workdir, name)
        }
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }
}

impl PlanBrowseHandler {
    /// List all plans in the workspace, formatted as a summary table.
    fn list_plans(&self, workdir: &std::path::Path) -> SlashResult {
        let summaries = match plan_file::list_plan_summaries(workdir) {
            Ok(s) => s,
            Err(e) => {
                return SlashResult::Reply(format!("读取 plan 列表失败：{e}"));
            }
        };

        if summaries.is_empty() {
            return SlashResult::Reply("当前工作区没有 plan。".to_owned());
        }

        let mut lines = Vec::with_capacity(summaries.len() + 1);
        lines.push(format!("找到 {} 个 plan：", summaries.len()));
        for s in &summaries {
            let mut line = format!(
                "  {} — {} {}/{} 完成",
                s.stem, s.title, s.completed, s.total
            );
            if s.failed > 0 {
                line.push_str(&format!(" {} 失败", s.failed));
            }
            if s.skipped > 0 {
                line.push_str(&format!(" {} 已跳过", s.skipped));
            }
            lines.push(line);
        }
        SlashResult::Reply(lines.join("\n"))
    }

    /// View the full content of a single plan by name.
    fn view_plan(&self, workdir: &std::path::Path, name: &str) -> SlashResult {
        let path = match plan_file::resolve_plan_by_name(workdir, name) {
            Ok(p) => p,
            Err(plan_file::PlanResolveError::NotFound { name }) => {
                return SlashResult::Reply(format!("未找到名为 \"{name}\" 的 plan。"));
            }
            Err(plan_file::PlanResolveError::Ambiguous { name, candidates }) => {
                let list = candidates.join(", ");
                return SlashResult::Reply(format!(
                    "\"{name}\" 匹配到多个 plan，请提供更精确的名称。候选：{list}"
                ));
            }
        };

        let full_path = workdir.join(&path);
        match plan_file::read_plan_content(&full_path) {
            Ok(content) => SlashResult::Reply(content),
            Err(e) => SlashResult::Reply(format!("读取 plan 文件失败：{e}")),
        }
    }
}
