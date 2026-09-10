//! Feishu tool definitions — individual sub-tool implementations.
//!
//! Registration is handled by [`ImAdapterToolsRegistrar`] in the
//! parent crate.  22 sub-tools correspond 1-to-1 with the tools
//! README "各模块注册的工具一览" table.

pub mod bitable;
pub mod calendar;
pub mod doc;
pub mod drive;
pub mod im;
pub mod sheet;
pub mod task;

#[cfg(test)]
mod tools_tests;

#[cfg(test)]
mod registrar_tests;

// im (4)
pub use im::FeishuImUserGetMessagesTool;
pub use im::FeishuImUserGetThreadMessagesTool;
pub use im::FeishuImUserMessageTool;
pub use im::FeishuSearchUserTool;

// calendar (4)
pub use calendar::FeishuCalendarCalendarTool;
pub use calendar::FeishuCalendarEventAttendeeTool;
pub use calendar::FeishuCalendarEventTool;
pub use calendar::FeishuCalendarFreebusyTool;

// task (4)
pub use task::FeishuTaskCommentTool;
pub use task::FeishuTaskSubtaskTool;
pub use task::FeishuTaskTaskTool;
pub use task::FeishuTaskTasklistTool;

// bitable (5)
pub use bitable::FeishuBitableAppTableFieldTool;
pub use bitable::FeishuBitableAppTableRecordTool;
pub use bitable::FeishuBitableAppTableTool;
pub use bitable::FeishuBitableAppTableViewTool;
pub use bitable::FeishuBitableAppTool;

// doc (3)
pub use doc::FeishuDocCommentsTool;
pub use doc::FeishuDocMediaTool;
pub use doc::FeishuSearchDocWikiTool;

// drive (1)
pub use drive::FeishuDriveFileTool;

// sheet (1)
pub use sheet::FeishuSheetTool;
