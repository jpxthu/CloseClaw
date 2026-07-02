//! Feishu tool registration — provides tool instances for registration
//! into the [`ToolRegistry`] at daemon startup.

pub mod bitable;
pub mod calendar;
pub mod doc;
pub mod drive;
pub mod im;
pub mod sheet;
pub mod task;

#[cfg(test)]
mod tools_tests;

use closeclaw_tools::Tool;

pub use bitable::FeishuBitableTool;
pub use calendar::FeishuCalendarTool;
pub use doc::FeishuDocTool;
pub use drive::FeishuDriveTool;
pub use im::FeishuImTool;
pub use sheet::FeishuSheetTool;
pub use task::FeishuTaskTool;

/// Create all Feishu tool instances.
///
/// Returns a vector of boxed tools ready for registration.
/// Each tool group is created with `is_deferred_by_default = true`
/// per the design doc requirement that all Feishu tools load lazily.
pub fn create_feishu_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(FeishuImTool::new()),
        Box::new(FeishuCalendarTool::new()),
        Box::new(FeishuTaskTool::new()),
        Box::new(FeishuBitableTool::new()),
        Box::new(FeishuDocTool::new()),
        Box::new(FeishuDriveTool::new()),
        Box::new(FeishuSheetTool::new()),
    ]
}

/// Register all Feishu tool groups into the provided registry.
///
/// Uses `closeclaw_tools::ToolRegistry` directly.
pub async fn register_tools(registry: &closeclaw_tools::ToolRegistry) {
    registry.register(FeishuImTool::new()).await.ok();
    registry.register(FeishuCalendarTool::new()).await.ok();
    registry.register(FeishuTaskTool::new()).await.ok();
    registry.register(FeishuBitableTool::new()).await.ok();
    registry.register(FeishuDocTool::new()).await.ok();
    registry.register(FeishuDriveTool::new()).await.ok();
    registry.register(FeishuSheetTool::new()).await.ok();
}
