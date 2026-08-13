//! Lazy-loading tool wrapper.
//!
//! [`LazyTool`] wraps a tool factory and defers actual tool creation
//! until the first [`Tool::call`].  Metadata methods (`name`, `group`,
//! `summary`, `detail`, `input_schema`, `flags`) return pre-stored
//! values so the tool can be registered without instantiating the real
//! tool.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::OnceCell;

use crate::tool_registry::ToolFlags;
use crate::tool_trait::{Tool, ToolCallError, ToolContext, ToolResult};

/// Pre-stored metadata for a tool, consumed by [`LazyTool::new`].
///
/// Grouping the metadata fields into one struct keeps
/// `LazyTool::new` at two parameters (factory + meta) while
/// remaining self-documenting.
pub struct ToolMeta {
    /// Tool name returned by [`Tool::name`].
    pub name: String,
    /// Tool group returned by [`Tool::group`].
    pub group: String,
    /// Short description returned by [`Tool::summary`].
    pub summary: String,
    /// Longer description returned by [`Tool::detail`].
    pub detail: String,
    /// JSON Schema for the tool's input.
    pub input_schema: Value,
    /// Runtime flags (e.g. `is_deferred_by_default`).
    pub flags: ToolFlags,
}

/// A lazy-loading wrapper around a [`Tool`] implementation.
///
/// Registration creates only a lightweight shell (metadata + factory).
/// The real tool is instantiated on the first [`Tool::call`] and
/// cached for subsequent invocations.
pub struct LazyTool {
    meta: ToolMeta,
    factory: Box<dyn Fn() -> Box<dyn Tool> + Send + Sync>,
    inner: OnceCell<Arc<dyn Tool>>,
}

impl LazyTool {
    /// Create a new lazy tool.
    ///
    /// - `factory` — called at most once, on the first `call()`.
    /// - `meta` — static metadata returned by the `Tool` trait methods.
    pub fn new(
        factory: Box<dyn Fn() -> Box<dyn Tool> + Send + Sync>,
        meta: ToolMeta,
    ) -> Self {
        Self {
            meta,
            factory,
            inner: OnceCell::new(),
        }
    }

    /// Ensure the inner tool is initialized (factory called).
    ///
    /// Returns a reference to the cached `Arc<dyn Tool>`.
    async fn ensure_init(&self) -> &Arc<dyn Tool> {
        self.inner
            .get_or_init(|| async {
                let tool = (self.factory)();
                Arc::from(tool)
            })
            .await
    }
}

#[async_trait]
impl Tool for LazyTool {
    fn name(&self) -> &str {
        &self.meta.name
    }

    fn group(&self) -> &str {
        &self.meta.group
    }

    fn summary(&self) -> String {
        self.meta.summary.clone()
    }

    fn detail(&self) -> String {
        self.meta.detail.clone()
    }

    fn input_schema(&self) -> Value {
        self.meta.input_schema.clone()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let inner = self.ensure_init().await;
        inner.call(args, ctx).await
    }

    fn flags(&self) -> ToolFlags {
        self.meta.flags
    }
}
