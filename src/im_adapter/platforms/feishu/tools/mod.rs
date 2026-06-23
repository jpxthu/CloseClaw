//! Feishu tool registration — wires all platform-specific tools
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

use crate::tools::ToolRegistry;
use bitable::FeishuBitableTool;
use calendar::FeishuCalendarTool;
use doc::FeishuDocTool;
use drive::FeishuDriveTool;
use im::FeishuImTool;
use sheet::FeishuSheetTool;
use task::FeishuTaskTool;

/// Register all Feishu tool groups into the provided registry.
///
/// Each tool group is registered with `is_deferred_by_default = true`
/// per the design doc requirement that all Feishu tools load lazily.
pub(crate) async fn register_tools(registry: &ToolRegistry) {
    registry.register(FeishuImTool::new()).await.ok();
    registry.register(FeishuCalendarTool::new()).await.ok();
    registry.register(FeishuTaskTool::new()).await.ok();
    registry.register(FeishuBitableTool::new()).await.ok();
    registry.register(FeishuDocTool::new()).await.ok();
    registry.register(FeishuDriveTool::new()).await.ok();
    registry.register(FeishuSheetTool::new()).await.ok();
}
