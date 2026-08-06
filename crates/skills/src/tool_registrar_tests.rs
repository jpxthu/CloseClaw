//! Tests for SkillsToolsRegistrar: name, priority, register, conflict.

use async_trait::async_trait;
use closeclaw_common::tool_registry::{
    RegistryError, ToolRegistrar, ToolRegistrarError, ToolRegistry, ToolRegistryQuery,
};
use closeclaw_common::tool_trait::{Tool, ToolFlags};
use closeclaw_common::ToolDescriptor;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::SkillsToolsRegistrar;

// ---------------------------------------------------------------------------
// Mock Tool — minimal satisfying implementation
// ---------------------------------------------------------------------------

struct MockTool {
    name: String,
}

impl MockTool {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn group(&self) -> &str {
        "test"
    }
    fn summary(&self) -> String {
        format!("mock tool {}", self.name)
    }
    fn detail(&self) -> String {
        format!("mock detail for {}", self.name)
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({})
    }
    fn flags(&self) -> ToolFlags {
        ToolFlags::default()
    }
}

// ---------------------------------------------------------------------------
// Mock ToolRegistry — records registrations, supports conflict injection
// ---------------------------------------------------------------------------

struct MockToolRegistry {
    /// Map of tool name → registrar_name that registered it.
    registered: Mutex<HashMap<String, String>>,
    /// When set, the next `register_any` call returns a Conflict error.
    conflict_tool: Mutex<Option<String>>,
}

impl MockToolRegistry {
    fn new() -> Self {
        Self {
            registered: Mutex::new(HashMap::new()),
            conflict_tool: Mutex::new(None),
        }
    }

    /// Set up the next registration to return a Conflict error.
    async fn set_conflict(&self, tool_name: &str) {
        *self.conflict_tool.lock().await = Some(tool_name.to_string());
    }

    /// Returns a snapshot of currently registered tools.
    async fn registered_tools(&self) -> Vec<String> {
        self.registered.lock().await.keys().cloned().collect()
    }
}

#[async_trait]
impl ToolRegistry for MockToolRegistry {
    async fn register_any(
        &self,
        tool: Box<dyn std::any::Any + Send + Sync>,
        registrar_name: &str,
    ) -> Result<(), RegistryError> {
        // Check if we should simulate a conflict.
        let conflict = self.conflict_tool.lock().await.take();
        if let Some(ref conflicting_name) = conflict {
            // Extract tool name from the ToolBox.
            let boxed = tool
                .downcast_ref::<closeclaw_common::tool_registry::ToolBox>()
                .expect("expected ToolBox");
            let tool_name = boxed.0.name().to_string();
            if tool_name == *conflicting_name {
                return Err(RegistryError::Conflict {
                    tool: tool_name,
                    registrar: "some-other-registrar".to_string(),
                    attempting: registrar_name.to_string(),
                });
            }
        }

        // Extract tool name and record registration.
        let boxed = tool
            .downcast_ref::<closeclaw_common::tool_registry::ToolBox>()
            .expect("expected ToolBox");
        let tool_name = boxed.0.name().to_string();
        self.registered
            .lock()
            .await
            .insert(tool_name, registrar_name.to_string());
        Ok(())
    }

    fn freeze(&self) {}
    fn is_frozen(&self) -> bool {
        false
    }
    async fn build_index(&self) -> String {
        String::new()
    }
}

#[async_trait]
impl ToolRegistryQuery for MockToolRegistry {
    async fn list_tool_names(&self) -> Vec<String> {
        self.registered_tools().await
    }
    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        _agent_tools: Option<&[String]>,
        _agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<ToolDescriptor> {
        vec![]
    }
    async fn has_tool(&self, _name: &str) -> bool {
        false
    }
    async fn get_tool_schema(&self, _name: &str) -> Option<Value> {
        None
    }
    async fn get_tool_detail(&self, _name: &str) -> Option<ToolDescriptor> {
        None
    }
    async fn list_tool_names_by_group(&self, _group: &str) -> Vec<String> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_skills_tools_registrar_name() {
    let tool = Arc::new(MockTool::new("test_tool"));
    let registrar = SkillsToolsRegistrar::new(tool);
    assert_eq!(registrar.name(), "SkillsToolsRegistrar");
}

#[test]
fn test_skills_tools_registrar_priority() {
    let tool = Arc::new(MockTool::new("test_tool"));
    let registrar = SkillsToolsRegistrar::new(tool);
    assert_eq!(registrar.priority(), 3);
}

#[tokio::test]
async fn test_skills_tools_registrar_registers_tool() {
    let tool = Arc::new(MockTool::new("my_skill_tool"));
    let registrar = SkillsToolsRegistrar::new(tool);
    let registry = MockToolRegistry::new();

    let result = registrar.register(&registry).await;
    assert!(
        result.is_ok(),
        "register() should succeed: {:?}",
        result.err()
    );

    let registered = registry.registered_tools().await;
    assert!(
        registered.contains(&"my_skill_tool".to_string()),
        "tool 'my_skill_tool' should be registered, got: {:?}",
        registered
    );
}

#[tokio::test]
async fn test_skills_tools_registrar_conflict() {
    let tool = Arc::new(MockTool::new("conflicting_tool"));
    let registrar = SkillsToolsRegistrar::new(tool);
    let registry = MockToolRegistry::new();

    // Set up the mock to return Conflict on the next registration.
    registry.set_conflict("conflicting_tool").await;

    let result = registrar.register(&registry).await;
    match result {
        Err(ToolRegistrarError::Conflict {
            tool,
            registrar: _,
            attempting,
        }) => {
            assert_eq!(tool, "conflicting_tool");
            assert_eq!(attempting, "SkillsToolsRegistrar");
        }
        other => panic!("expected ToolRegistrarError::Conflict, got: {:?}", other),
    }
}
