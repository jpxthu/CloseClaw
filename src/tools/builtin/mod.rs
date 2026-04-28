//! Built-in tools module.
//!
//! Re-exports all builtin tool implementations and provides a single
//! registration entry point.

pub mod file_ops;
pub mod permission;
pub mod search;

pub use file_ops::{EditTool, GrepTool, LsTool, ReadTool, WriteTool};
pub use permission::PermissionQueryTool;
pub use search::ToolSearchTool;

/// Registers all built-in tools with the given registry.
///
/// Currently registers the 5 file_ops tools:
/// - [`ReadTool`]
/// - [`WriteTool`]
/// - [`EditTool`]
/// - [`GrepTool`]
/// - [`LsTool`]
///
/// And 2 meta tools:
/// - [`ToolSearchTool`]
/// - [`PermissionQueryTool`]
pub async fn register_builtin_tools(registry: &crate::tools::ToolRegistry) {
    // file_ops
    registry.register(ReadTool::new()).await.ok();
    registry.register(WriteTool::new()).await.ok();
    registry.register(EditTool::new()).await.ok();
    registry.register(GrepTool::new()).await.ok();
    registry.register(LsTool::new()).await.ok();
    // meta
    registry.register(ToolSearchTool::new()).await.ok();
    registry.register(PermissionQueryTool::new()).await.ok();
}
