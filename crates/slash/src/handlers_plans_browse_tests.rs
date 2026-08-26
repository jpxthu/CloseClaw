//! Unit tests for PlanBrowseHandler.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_plans_browse::PlanBrowseHandler;
use closeclaw_common::plan_state::PlanState;
use closeclaw_common::session_lookup::PendingMessage;
use closeclaw_common::slash_router::SlashResult;
use closeclaw_common::{SessionMode, SlashSessionQuery};

// ── Mock ────────────────────────────────────────────────────────────────

/// Minimal mock implementing [`SlashSessionQuery`] for handler tests.
/// Only `get_workdir` is meaningfully implemented; all other methods
/// return defaults / are unimplemented (the handler under test only
/// calls `get_workdir`).
struct MockQuery {
    workdirs: std::sync::Mutex<std::collections::HashMap<String, PathBuf>>,
}

impl MockQuery {
    fn new() -> Self {
        Self {
            workdirs: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn set_workdir(&self, session_id: &str, path: PathBuf) {
        self.workdirs
            .lock()
            .unwrap()
            .insert(session_id.to_string(), path);
    }
}

#[async_trait]
impl SlashSessionQuery for MockQuery {
    async fn get_workdir(&self, session_id: &str) -> Option<PathBuf> {
        self.workdirs.lock().unwrap().get(session_id).cloned()
    }

    async fn set_workdir(&self, session_id: &str, path: PathBuf) {
        self.workdirs
            .lock()
            .unwrap()
            .insert(session_id.to_string(), path);
    }

    // ── All other methods: unimplemented (not called by handler) ────────

    async fn get_plan_state(&self, _: &str) -> Option<PlanState> {
        unimplemented!()
    }
    async fn set_plan_state(&self, _: &str, _: PlanState) {
        unimplemented!()
    }
    async fn push_pending_message(&self, _: &str, _: PendingMessage) -> Result<(), String> {
        unimplemented!()
    }
    async fn trigger_manual_background(&self, _: &str) -> Result<bool, String> {
        unimplemented!()
    }
    async fn set_workflow_run(
        &self,
        _: &str,
        _: Option<Box<dyn std::any::Any + Send + Sync>>,
    ) -> Result<(), String> {
        unimplemented!()
    }
    async fn invalidate_static_cache(&self) {
        unimplemented!()
    }
    async fn rebuild_system_prompt_for_session(&self, _: &str) {
        unimplemented!()
    }
    async fn add_system_append(&self, _: &str, _: String) {
        unimplemented!()
    }
    async fn get_model(&self, _: &str) -> Option<String> {
        unimplemented!()
    }
    async fn get_reasoning_level(&self, _: &str) -> Option<String> {
        unimplemented!()
    }
    async fn get_verbosity_level(&self, _: &str) -> Option<String> {
        unimplemented!()
    }
    async fn get_session_mode(&self, _: &str) -> Option<SessionMode> {
        unimplemented!()
    }
    async fn get_system_appends(&self, _: &str) -> Vec<String> {
        unimplemented!()
    }
    async fn is_llm_busy(&self, _: &str) -> bool {
        unimplemented!()
    }
    async fn get_stats(&self, _: &str) -> Option<(usize, usize, usize, usize)> {
        unimplemented!()
    }
    async fn get_last_cache_break(&self, _: &str) -> Option<String> {
        unimplemented!()
    }
    async fn get_active_child_count(&self, _: &str) -> usize {
        unimplemented!()
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn dummy_ctx() -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: "test_session".to_owned(),
        channel: "test_channel".to_owned(),
    }
}

fn make_handler(workdirs: std::collections::HashMap<String, PathBuf>) -> PlanBrowseHandler {
    let mock = Arc::new(MockQuery::new());
    for (sid, wd) in workdirs {
        mock.set_workdir(&sid, wd);
    }
    PlanBrowseHandler::new(mock as Arc<dyn SlashSessionQuery>)
}

fn make_handler_with_workdir(session_id: &str, workdir: PathBuf) -> PlanBrowseHandler {
    let mock = Arc::new(MockQuery::new());
    mock.set_workdir(session_id, workdir);
    PlanBrowseHandler::new(mock as Arc<dyn SlashSessionQuery>)
}

/// Create a plan file with content under `{workdir}/plans/{stem}.md`.
fn write_plan(workdir: &std::path::Path, stem: &str, content: &str) {
    let plans = workdir.join("plans");
    std::fs::create_dir_all(&plans).unwrap();
    std::fs::write(plans.join(format!("{stem}.md")), content).unwrap();
}

/// Set the modification time of a file.
fn set_mtime(path: &std::path::Path, seconds: u64) {
    let time = filetime::FileTime::from_unix_time(seconds as i64, 0);
    filetime::set_file_mtime(path, time).unwrap();
}

// ── Metadata tests ──────────────────────────────────────────────────────

#[test]
fn test_handler_commands_and_description() {
    let h = make_handler(std::collections::HashMap::new());
    assert_eq!(h.commands(), &["plans"]);
    assert_eq!(h.description(), "列出或查看工作区中的 plan");
}

#[test]
fn test_handler_not_immediate() {
    let h = make_handler(std::collections::HashMap::new());
    assert!(!h.immediate("plans"));
}

// ── /plans (no args) — list ─────────────────────────────────────────────

#[tokio::test]
async fn test_list_plans_no_workdir() {
    let h = make_handler(std::collections::HashMap::new());
    let ctx = dummy_ctx();
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("未设置工作目录"),
                "expected no-workdir message, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_list_plans_empty_workdir_no_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("没有 plan"),
                "expected empty message, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_list_plans_empty_workdir_plans_dir_exists() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("plans")).unwrap();
    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("没有 plan"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_list_plans_single_plan() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(
        dir.path(),
        "alpha",
        "# Alpha Feature\n\n## Tasks\n\n- [x] Step 1\n- [ ] Step 2\n",
    );
    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("1 个 plan"), "got: {text}");
            assert!(text.contains("alpha"), "should show stem");
            assert!(text.contains("Alpha Feature"), "should show title");
            assert!(
                text.contains("1/2 完成"),
                "should show 1/2 完成, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_list_plans_sorted_by_mtime_desc() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(dir.path(), "old-plan", "# Old Plan\n");
    write_plan(dir.path(), "new-plan", "# New Plan\n");

    let old_path = dir.path().join("plans/old-plan.md");
    let new_path = dir.path().join("plans/new-plan.md");
    set_mtime(&old_path, 1000);
    set_mtime(&new_path, 2000);

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("2 个 plan"), "got: {text}");
            // new-plan should appear before old-plan in the listing
            let new_pos = text.find("new-plan").unwrap_or(usize::MAX);
            let old_pos = text.find("old-plan").unwrap_or(usize::MAX);
            assert!(
                new_pos < old_pos,
                "new-plan should appear before old-plan (new_pos={new_pos}, old_pos={old_pos})"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_list_plans_shows_completion_counts() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(
        dir.path(),
        "plan-a",
        "# Plan A\n\n## Tasks\n\n- [x] Done\n- [!] Critical done\n- [~] In progress\n- [ ] Pending\n",
    );
    write_plan(
        dir.path(),
        "plan-b",
        "# Plan B\n\n## Tasks\n\n- [ ] Only pending\n",
    );

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("1/4 完成"),
                "plan-a should show 1/4 完成, got: {text}"
            );
            assert!(
                text.contains("1 失败"),
                "plan-a should show 1 失败, got: {text}"
            );
            assert!(
                text.contains("1 已跳过"),
                "plan-a should show 1 已跳过, got: {text}"
            );
            assert!(
                text.contains("0/1 完成"),
                "plan-b should show 0/1 完成, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_list_plans_ignores_non_md_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans = dir.path().join("plans");
    std::fs::create_dir_all(&plans).unwrap();
    std::fs::write(plans.join("notes.txt"), "not a plan").unwrap();
    write_plan(dir.path(), "real", "# Real Plan\n");

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("1 个 plan"), "got: {text}");
            assert!(text.contains("real"), "should include real plan");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── /plans <name> — view ────────────────────────────────────────────────

#[tokio::test]
async fn test_view_plan_exact_match() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(dir.path(), "my-plan", "# My Plan\n\nContent here.");

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("my-plan", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("# My Plan"), "should show full content");
            assert!(text.contains("Content here."));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_view_plan_prefix_match() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(dir.path(), "2026-08-19-auth", "# Auth Feature\n");

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("2026", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("# Auth Feature"));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_view_plan_fuzzy_match() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(dir.path(), "2026-08-19-auth-feature", "# Auth Feature\n");

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("auth", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("# Auth Feature"));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_view_plan_not_found() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(dir.path(), "existing", "# Existing\n");

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("nonexistent", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("未找到"),
                "expected not-found message, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_view_plan_ambiguous() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(dir.path(), "auth-login", "# Auth Login\n");
    write_plan(dir.path(), "auth-logout", "# Auth Logout\n");

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("auth", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("匹配到多个"),
                "expected ambiguous message, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_view_plan_no_workdir() {
    let h = make_handler(std::collections::HashMap::new());
    let ctx = dummy_ctx();
    match h.handle("anything", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("未设置工作目录"), "got: {text}");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_view_plan_chinese_title() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(
        dir.path(),
        "zh-plan",
        "# 实现用户认证功能\n\n## Tasks\n\n- [x] 设计 API\n- [ ] 编写测试\n",
    );

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("zh-plan", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("实现用户认证功能"));
            assert!(text.contains("设计 API"));
            assert!(text.contains("编写测试"));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_list_plan_chinese_title() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(
        dir.path(),
        "zh-plan",
        "# 实现用户认证功能\n\n## Tasks\n\n- [x] Done\n",
    );

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("实现用户认证功能"));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_view_plan_no_tasks_section() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(
        dir.path(),
        "no-tasks",
        "# No Tasks\n\n## Context\n\nJust context.",
    );

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("no-tasks", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("# No Tasks"));
            assert!(text.contains("Just context."));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_list_plan_no_tasks_section() {
    let dir = tempfile::TempDir::new().unwrap();
    write_plan(dir.path(), "no-tasks", "# No Tasks\n");

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("0/0"), "no-tasks should show 0/0");
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_view_plan_file_not_readable() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans = dir.path().join("plans");
    std::fs::create_dir_all(&plans).unwrap();
    // Create a file, then make it unreadable
    let bad_path = plans.join("bad.md");
    std::fs::write(&bad_path, "content").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bad_path, std::fs::Permissions::from_mode(0o000)).unwrap();
    }

    let h = make_handler_with_workdir("test_session", dir.path().to_path_buf());
    let ctx = dummy_ctx();
    match h.handle("bad", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("读取") || text.contains("失败"),
                "expected read error, got: {text}"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }

    // Restore permissions for cleanup
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bad_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
}
