//! ToolRegistrar — 模块级工具注册 trait
//!
//! 每个工具提供模块实现此 trait，通过统一的 `register_all` 编排完成全局注册。
//! 参见 `docs/design/tools/tool-registrar.md`。

// Re-export so that `crate::registrar::ToolRegistrar` still resolves.
pub use closeclaw_common::tool_registry::ToolRegistrar;

// Re-export registration helpers from common.
pub use closeclaw_common::tool_registry::{register_single, register_tool};

/// Register a tool and increment the counter on success.
///
/// Used inside registrar `register()` methods to reduce boilerplate.
/// Extracts the tool name, calls [`register_single`], and increments
/// `$registered` when the tool is accepted.
#[macro_export]
macro_rules! try_register {
    ($registry:expr, $registered:expr, $tool:expr, $registrar_name:expr) => {
        let tool = $tool;
        let name = tool.name().to_string();
        if closeclaw_common::tool_registry::register_single($registry, name, tool, $registrar_name)
            .await?
        {
            $registered += 1;
        }
    };
}
