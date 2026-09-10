//! Tests for `register_before_freeze` method on ToolRegistryImpl.

use super::*;
use std::sync::Arc;

/// Normal registration: tool can be queried after `register_before_freeze`.
#[tokio::test]
async fn test_register_before_freeze_normal() {
    let reg = ToolRegistry::new();
    let tool = Arc::new(DummyTool {
        name: "ModeTrigger".to_string(),
        group: "mode".to_string(),
        summary_text: "mode trigger".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    });

    reg.register_before_freeze(tool, "SystemLevel")
        .await
        .unwrap();

    let detail = reg.get_detail("ModeTrigger").await.unwrap();
    assert!(detail.contains("ModeTrigger"));
}

/// Frozen registry: `register_before_freeze` returns `ToolError::Frozen`.
#[tokio::test]
async fn test_register_before_freeze_frozen() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "read".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    // Freeze via register_all (single registrar)
    let registrar = make_simple_registrar(vec![]);
    reg.register_all(vec![Box::new(registrar)]).await.unwrap();
    assert!(reg.is_frozen());

    let tool = Arc::new(DummyTool {
        name: "WorkflowStart".to_string(),
        group: "workflow".to_string(),
        summary_text: "start".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    });

    let err = reg
        .register_before_freeze(tool, "SystemLevel")
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Frozen));
}

/// Duplicate tool name: `register_before_freeze` returns
/// `ToolError::AlreadyRegistered`.
#[tokio::test]
async fn test_register_before_freeze_duplicate() {
    let reg = ToolRegistry::new();
    let tool1 = Arc::new(DummyTool {
        name: "WorkflowStart".to_string(),
        group: "workflow".to_string(),
        summary_text: "start v1".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    });
    reg.register_before_freeze(tool1, "SystemLevel")
        .await
        .unwrap();

    let tool2 = Arc::new(DummyTool {
        name: "WorkflowStart".to_string(),
        group: "workflow".to_string(),
        summary_text: "start v2".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    });
    let err = reg
        .register_before_freeze(tool2, "SystemLevel")
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::AlreadyRegistered(_)));
}

/// After `register_before_freeze`, the generation counter increments.
#[tokio::test]
async fn test_register_before_freeze_increments_generation() {
    let reg = ToolRegistry::new();
    let gen_before = reg.generation();

    let tool = Arc::new(DummyTool {
        name: "ModeTrigger".to_string(),
        group: "mode".to_string(),
        summary_text: "trigger".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    });
    reg.register_before_freeze(tool, "SystemLevel")
        .await
        .unwrap();

    assert_eq!(reg.generation(), gen_before + 1);
}

/// `register_before_freeze` sets the owner correctly.
#[tokio::test]
async fn test_register_before_freeze_records_owner() {
    let reg = ToolRegistry::new();
    let tool = Arc::new(DummyTool {
        name: "WorkflowStart".to_string(),
        group: "workflow".to_string(),
        summary_text: "start".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    });
    reg.register_before_freeze(tool, "WorkflowRegistrar")
        .await
        .unwrap();

    let guard = reg.owners.read().await;
    assert_eq!(
        guard.get("WorkflowStart"),
        Some(&"WorkflowRegistrar".to_string())
    );
}

/// Multiple `register_before_freeze` calls before freeze works correctly.
#[tokio::test]
async fn test_register_before_freeze_multiple_then_freeze() {
    use closeclaw_common::ToolRegistry as ToolRegistryTrait;
    let reg = ToolRegistry::new();
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(DummyTool {
            name: "ModeTrigger".to_string(),
            group: "mode".to_string(),
            summary_text: "mode".to_string(),
            is_deferred: false,
            is_read_only: false,
            is_destructive: false,
        }),
        Arc::new(DummyTool {
            name: "WorkflowStart".to_string(),
            group: "workflow".to_string(),
            summary_text: "ws".to_string(),
            is_deferred: false,
            is_read_only: false,
            is_destructive: false,
        }),
        Arc::new(DummyTool {
            name: "WorkflowVerify".to_string(),
            group: "workflow".to_string(),
            summary_text: "wv".to_string(),
            is_deferred: false,
            is_read_only: false,
            is_destructive: false,
        }),
    ];

    for tool in tools {
        reg.register_before_freeze(tool, "SystemLevel")
            .await
            .unwrap();
    }

    // All tools queryable
    assert!(reg.get_detail("ModeTrigger").await.is_ok());
    assert!(reg.get_detail("WorkflowStart").await.is_ok());
    assert!(reg.get_detail("WorkflowVerify").await.is_ok());

    // Now freeze via trait method
    ToolRegistryTrait::freeze(&reg);
    assert!(reg.is_frozen());

    // After freeze, registration fails
    let tool = Arc::new(DummyTool {
        name: "Extra".to_string(),
        group: "extra".to_string(),
        summary_text: "extra".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    });
    let err = reg
        .register_before_freeze(tool, "SystemLevel")
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Frozen));
}

/// After `register_before_freeze`, multiple tools can be registered
/// and then frozen via the trait method.
#[tokio::test]
async fn test_register_before_freeze_then_freeze_via_trait() {
    use closeclaw_common::ToolRegistry as ToolRegistryTrait;
    let reg = ToolRegistry::new();
    let tool = Arc::new(DummyTool {
        name: "ModeTrigger".to_string(),
        group: "mode".to_string(),
        summary_text: "trigger".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    });
    reg.register_before_freeze(tool, "SystemLevel")
        .await
        .unwrap();

    // Freeze via trait method
    ToolRegistryTrait::freeze(&reg);
    assert!(reg.is_frozen());

    // After freeze, registration fails
    let tool2 = Arc::new(DummyTool {
        name: "Extra".to_string(),
        group: "extra".to_string(),
        summary_text: "extra".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    });
    let err = reg
        .register_before_freeze(tool2, "SystemLevel")
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Frozen));
}

/// Helper: create a simple ToolRegistrar that registers no tools.
fn make_simple_registrar(tools: Vec<Arc<dyn Tool>>) -> SimpleRegistrar {
    SimpleRegistrar(tools)
}

struct SimpleRegistrar(Vec<Arc<dyn Tool>>);

#[async_trait::async_trait]
impl closeclaw_common::tool_registry::ToolRegistrar for SimpleRegistrar {
    fn name(&self) -> &str {
        "SimpleRegistrar"
    }
    fn priority(&self) -> u32 {
        99
    }
    async fn register(
        &self,
        registry: &dyn closeclaw_common::tool_registry::ToolRegistry,
    ) -> Result<(), closeclaw_common::tool_registry::ToolRegistrarError> {
        for tool in &self.0 {
            let boxed: Box<dyn std::any::Any + Send + Sync> =
                Box::new(closeclaw_common::tool_registry::ToolBox(tool.clone()));
            registry
                .register_any(boxed, "SimpleRegistrar")
                .await
                .map_err(|e| match e {
                    closeclaw_common::tool_registry::RegistryError::Conflict {
                        tool,
                        registrar,
                        attempting,
                    } => closeclaw_common::tool_registry::ToolRegistrarError::Conflict {
                        tool,
                        registrar,
                        attempting,
                    },
                    other => closeclaw_common::tool_registry::ToolRegistrarError::Internal(
                        other.to_string(),
                    ),
                })?;
        }
        Ok(())
    }
}
