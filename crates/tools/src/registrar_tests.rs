//! Tests for ToolRegistrar trait, register_all, freeze, and all registrars.
//!
//! Covers:
//! - register_all with multiple registrars (priority-ordered)
//! - register_all with single registrar
//! - register_all with empty Vec
//! - error propagation when a registrar fails
//! - freeze: register_all freezes the registry, subsequent register returns error
//! - each registrar registers the correct tools
//! - priority ordering
//! - conflict detection (same tool name from two registrars)

use async_trait::async_trait;

use crate::registrar::{ToolRegistrar, ToolRegistrarError};
use crate::{Tool, ToolContext, ToolFlags, ToolRegistry};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Dummy helpers
// ---------------------------------------------------------------------------

struct DummyTool {
    name: String,
    group: String,
    is_deferred: bool,
}

impl Tool for DummyTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn group(&self) -> &str {
        &self.group
    }
    fn summary(&self) -> String {
        format!("dummy {}", self.name)
    }
    fn detail(&self) -> String {
        format!("detail for {}", self.name)
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    fn flags(&self) -> ToolFlags {
        let mut f = ToolFlags::default();
        f.is_deferred_by_default = self.is_deferred;
        f
    }
}

/// A test registrar that registers a fixed set of dummy tools.
struct TestRegistrar {
    name: String,
    priority: u32,
    tools: Vec<(String, String, bool)>, // (name, group, is_deferred)
}

impl TestRegistrar {
    fn new(name: &str, priority: u32, tools: Vec<(&str, &str, bool)>) -> Self {
        Self {
            name: name.to_string(),
            priority,
            tools: tools
                .into_iter()
                .map(|(n, g, d)| (n.to_string(), g.to_string(), d))
                .collect(),
        }
    }
}

#[async_trait]
impl ToolRegistrar for TestRegistrar {
    fn name(&self) -> &str {
        &self.name
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    async fn register(&self, registry: &ToolRegistry) -> Result<(), ToolRegistrarError> {
        for (name, group, deferred) in &self.tools {
            registry
                .register(DummyTool {
                    name: name.clone(),
                    group: group.clone(),
                    is_deferred: *deferred,
                })
                .await
                .map_err(|e| match e {
                    crate::ToolError::AlreadyRegistered(n) => ToolRegistrarError::Conflict {
                        tool: n,
                        registrar: self.name.clone(),
                    },
                    other => ToolRegistrarError::Internal(other.to_string()),
                })?;
        }
        Ok(())
    }
}

/// A test registrar that always fails on register.
struct FailingRegistrar {
    name: String,
    priority: u32,
}

#[async_trait]
impl ToolRegistrar for FailingRegistrar {
    fn name(&self) -> &str {
        &self.name
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    async fn register(&self, _registry: &ToolRegistry) -> Result<(), ToolRegistrarError> {
        Err(ToolRegistrarError::Internal(
            "intentional failure".to_string(),
        ))
    }
}

fn make_ctx() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
    }
}

// ---------------------------------------------------------------------------
// register_all — normal path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_all_multiple_registrars() {
    let registry = ToolRegistry::new();
    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![
        Box::new(TestRegistrar::new(
            "B",
            2,
            vec![("ToolB", "group_b", false)],
        )),
        Box::new(TestRegistrar::new(
            "A",
            1,
            vec![("ToolA", "group_a", false)],
        )),
    ];

    registry.register_all(registrars).await.unwrap();

    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;
    assert_eq!(descriptors.len(), 2);
    assert!(descriptors.iter().any(|d| d.name == "ToolA"));
    assert!(descriptors.iter().any(|d| d.name == "ToolB"));
}

#[tokio::test]
async fn test_register_all_single_registrar() {
    let registry = ToolRegistry::new();
    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![Box::new(TestRegistrar::new(
        "Solo",
        0,
        vec![("SoloTool", "solo_group", false)],
    ))];

    registry.register_all(registrars).await.unwrap();

    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].name, "SoloTool");
}

#[tokio::test]
async fn test_register_all_empty_vec() {
    let registry = ToolRegistry::new();
    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![];

    registry.register_all(registrars).await.unwrap();

    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;
    assert!(descriptors.is_empty());
}

// ---------------------------------------------------------------------------
// register_all — error path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_all_registrar_failure() {
    let registry = ToolRegistry::new();
    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![
        Box::new(TestRegistrar::new(
            "OK",
            1,
            vec![("OkTool", "ok_group", false)],
        )),
        Box::new(FailingRegistrar {
            name: "FailRegistrar".to_string(),
            priority: 2,
        }),
    ];

    let result = registry.register_all(registrars).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolRegistrarError::Internal(msg) => assert_eq!(msg, "intentional failure"),
        other => panic!("expected Internal error, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Freeze behavior
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_all_freezes_registry() {
    let registry = ToolRegistry::new();
    assert!(!registry.is_frozen());

    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![Box::new(TestRegistrar::new(
        "R",
        0,
        vec![("T1", "g", false)],
    ))];

    registry.register_all(registrars).await.unwrap();
    assert!(registry.is_frozen());
}

#[tokio::test]
async fn test_register_rejected_after_freeze() {
    let registry = ToolRegistry::new();
    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![Box::new(TestRegistrar::new(
        "R",
        0,
        vec![("T1", "g", false)],
    ))];

    registry.register_all(registrars).await.unwrap();

    let err = registry
        .register(DummyTool {
            name: "Extra".to_string(),
            group: "g".to_string(),
            is_deferred: false,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, crate::ToolError::Frozen));
}

#[tokio::test]
async fn test_register_all_rejected_on_frozen_registry() {
    let registry = ToolRegistry::new();
    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![Box::new(TestRegistrar::new(
        "R",
        0,
        vec![("T1", "g", false)],
    ))];

    registry.register_all(registrars).await.unwrap();

    let result = registry
        .register_all(vec![Box::new(TestRegistrar::new(
            "R2",
            0,
            vec![("T2", "g2", false)],
        ))])
        .await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Conflict detection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_all_conflict_detection() {
    let registry = ToolRegistry::new();
    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![
        Box::new(TestRegistrar::new(
            "R1",
            1,
            vec![("SameName", "group1", false)],
        )),
        Box::new(TestRegistrar::new(
            "R2",
            2,
            vec![("SameName", "group2", false)],
        )),
    ];

    let result = registry.register_all(registrars).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolRegistrarError::Conflict { tool, registrar } => {
            assert_eq!(tool, "SameName");
            assert_eq!(registrar, "R2");
        }
        other => panic!("expected Conflict error, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Priority ordering
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_all_priority_ordering() {
    /// A registrar that records the order in which it was called.
    struct OrderRecorder {
        name: String,
        priority: u32,
        order_log: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ToolRegistrar for OrderRecorder {
        fn name(&self) -> &str {
            &self.name
        }

        fn priority(&self) -> u32 {
            self.priority
        }

        async fn register(&self, _registry: &ToolRegistry) -> Result<(), ToolRegistrarError> {
            self.order_log.lock().unwrap().push(self.name.clone());
            Ok(())
        }
    }

    let order_log = Arc::new(Mutex::new(Vec::new()));
    let registry = ToolRegistry::new();

    // Pass them in reverse priority order — register_all should sort by priority.
    let registrars: Vec<Box<dyn ToolRegistrar>> = vec![
        Box::new(OrderRecorder {
            name: "HighPriority".to_string(),
            priority: 10,
            order_log: Arc::clone(&order_log),
        }),
        Box::new(OrderRecorder {
            name: "LowPriority".to_string(),
            priority: 1,
            order_log: Arc::clone(&order_log),
        }),
        Box::new(OrderRecorder {
            name: "MidPriority".to_string(),
            priority: 5,
            order_log: Arc::clone(&order_log),
        }),
    ];

    registry.register_all(registrars).await.unwrap();

    let recorded = order_log.lock().unwrap().clone();
    assert_eq!(
        recorded.len(),
        3,
        "all three registrars must have been called"
    );
    assert_eq!(
        recorded[0], "LowPriority",
        "priority 1 should be called first"
    );
    assert_eq!(
        recorded[1], "MidPriority",
        "priority 5 should be called second"
    );
    assert_eq!(
        recorded[2], "HighPriority",
        "priority 10 should be called third"
    );
}
