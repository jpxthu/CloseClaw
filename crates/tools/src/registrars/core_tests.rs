//! Unit tests for the core tools registrar — injection-chain tests.

use super::*;
use crate::plan_file_access::PlanResolveError;
use crate::ToolContext;
use closeclaw_common::permission_types::RiskLevel;
use closeclaw_tasks::BackgroundTaskManager;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

/// Permission port that allows everything — `call_tool` bypasses
/// permission checks anyway, so the registrar constructor is the only
/// consumer of this dependency.
struct AllowAllPermCheck;

#[async_trait]
impl crate::permission_port::ToolPermissionCheck for AllowAllPermCheck {
    async fn evaluate(
        &self,
        _session_id: Option<&str>,
        _caller: &mut crate::permission_port::PermCaller,
        _body: &crate::permission_port::PermRequestBody,
    ) -> crate::permission_port::PermVerdict {
        crate::permission_port::PermVerdict::Allowed
    }

    async fn submit_denial(
        &self,
        _caller: &crate::permission_port::PermCaller,
        _body: &crate::permission_port::PermRequestBody,
        _risk_level: RiskLevel,
        _session_id: &str,
        _is_sub_agent: bool,
    ) -> Option<String> {
        None
    }

    async fn is_session_sub_agent(&self, _session_id: &str) -> bool {
        false
    }

    fn is_config_file(&self, _path: &str) -> bool {
        false
    }
}

/// Recorder fake port: captures `(path, content)` pairs handed to
/// `write_plan_file`.
#[derive(Default)]
struct FakePlanAccess {
    writes: std::sync::Mutex<Vec<(PathBuf, String)>>,
}

impl FakePlanAccess {
    fn writes(&self) -> Vec<(PathBuf, String)> {
        self.writes.lock().unwrap().clone()
    }
}

impl PlanFileAccess for FakePlanAccess {
    fn resolve_plan_by_name(
        &self,
        _workdir: &Path,
        name: &str,
    ) -> Result<PathBuf, PlanResolveError> {
        Err(PlanResolveError::NotFound {
            name: name.to_string(),
        })
    }

    fn touch_access_timestamp(&self, _plan_path: &Path) -> Result<(), std::io::Error> {
        Ok(())
    }

    fn write_plan_file(&self, plan_path: &Path, content: &str) -> Result<(), std::io::Error> {
        self.writes
            .lock()
            .unwrap()
            .push((plan_path.to_path_buf(), content.to_string()));
        Ok(())
    }
}

/// Build a registrar wired with `fake` and register all 14 core tools
/// into a fresh registry; returns the registry for `call_tool` probing.
async fn registered_registry(fake: Arc<FakePlanAccess>) -> Arc<crate::ToolRegistry> {
    let registry = Arc::new(crate::ToolRegistry::new());
    let registrar = CoreToolsRegistrar::new(
        Arc::new(AllowAllPermCheck) as PermDeps,
        Arc::new(BackgroundTaskManager::new()) as Arc<dyn TaskManager>,
        Arc::clone(&registry) as Arc<dyn ToolRegistryQuery>,
    )
    .with_plan_file_access(fake);
    registrar
        .register(registry.as_ref())
        .await
        .expect("core registrar must register its tools");
    registry
}

fn ctx_with_workdir(workdir: &Path) -> ToolContext {
    ToolContext {
        agent_id: "master".to_string(),
        workdir: Some(crate::build_workdir_context(workdir.to_str().unwrap())),
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

// ---------------------------------------------------------------------------
// Builder pass-through tests
// ---------------------------------------------------------------------------

/// The `with_plan_file_access` builder must reach BOTH registered tools:
/// after `register`, plans/ Write and Edit calls through `call_tool`
/// must hit the injected port with the workdir-normalized path.
#[tokio::test]
async fn test_builder_passes_plan_file_access_to_write_and_edit() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("plans")).unwrap();
    std::fs::write(tmp.path().join("plans/x.md"), "keep me\n").unwrap();

    let fake = Arc::new(FakePlanAccess::default());
    let registry = registered_registry(Arc::clone(&fake)).await;
    let ctx = ctx_with_workdir(tmp.path());

    registry
        .call_tool(
            "Write",
            serde_json::json!({ "path": "plans/new.md", "content": "body" }),
            &ctx,
        )
        .await
        .expect("plans/ Write must go through the injected port");

    registry
        .call_tool(
            "Edit",
            serde_json::json!({
                "path": "plans/x.md",
                "edits": [{ "oldText": "me", "newText": "us" }]
            }),
            &ctx,
        )
        .await
        .expect("plans/ Edit must go through the injected port");

    let writes = fake.writes();
    assert_eq!(
        writes.len(),
        2,
        "both tools must route plans/ writes through the same port"
    );
    assert_eq!(
        writes[0],
        (tmp.path().join("plans/new.md"), "body".to_string(),),
        "Write must hand the normalized path and content to the port"
    );
    assert_eq!(
        writes[1],
        (tmp.path().join("plans/x.md"), "keep us\n".to_string(),),
        "Edit must hand the normalized path and replaced content to the port"
    );
}

/// Default wiring (no builder call) keeps the fail-fast contract: a
/// plans/ Write through the registrar errors instead of silently
/// falling back to the unlocked write path.
#[tokio::test]
async fn test_registrar_without_port_keeps_plans_write_fail_fast() {
    let tmp = TempDir::new().unwrap();
    let registry = Arc::new(crate::ToolRegistry::new());
    let registrar = CoreToolsRegistrar::new(
        Arc::new(AllowAllPermCheck) as PermDeps,
        Arc::new(BackgroundTaskManager::new()) as Arc<dyn TaskManager>,
        Arc::clone(&registry) as Arc<dyn ToolRegistryQuery>,
    );
    registrar
        .register(registry.as_ref())
        .await
        .expect("core registrar must register its tools");

    let ctx = ctx_with_workdir(tmp.path());
    let result = registry
        .call_tool(
            "Write",
            serde_json::json!({ "path": "plans/x.md", "content": "orphan" }),
            &ctx,
        )
        .await;
    match result {
        Err(crate::ToolCallError::ExecutionFailed(msg)) => {
            assert!(
                msg.contains("PlanFileAccess"),
                "error must explain the missing port, got: {msg}"
            );
        }
        other => panic!("expected fail-fast ExecutionFailed, got {other:?}"),
    }
    assert!(
        !tmp.path().join("plans").exists(),
        "fail-fast must not create any file"
    );
}
