//! Bridge implementations — adapts closeclaw-slash concrete types to
//! `closeclaw_common` trait objects used by the gateway.

use std::sync::Arc;

use async_trait::async_trait;

use closeclaw_slash::SlashDispatcher;

/// Newtype wrapper around `SlashDispatcher` to satisfy the orphan rule
/// when implementing `closeclaw_common::slash_router::SlashRouter`.
pub struct SlashDispatcherWrapper(pub SlashDispatcher);

/// Thin wrapper converting `Arc<dyn SlashHandler>` to `Box<dyn SlashHandler>`
/// for the common `SlashRouter` trait.
struct SlashHandlerBox {
    inner: Arc<dyn closeclaw_common::slash_router::SlashHandler>,
}

#[async_trait]
impl closeclaw_common::slash_router::SlashHandler for SlashHandlerBox {
    fn commands(&self) -> &[&str] {
        self.inner.commands()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn immediate(&self, cmd: &str) -> bool {
        self.inner.immediate(cmd)
    }
    fn requires_permission(&self) -> bool {
        self.inner.requires_permission()
    }
    async fn handle(
        &self,
        args: &str,
        ctx: &closeclaw_common::slash_router::SlashContext,
    ) -> closeclaw_common::slash_router::SlashResult {
        self.inner.handle(args, ctx).await
    }
}

#[async_trait]
impl closeclaw_common::slash_router::SlashRouter for SlashDispatcherWrapper {
    async fn dispatch(
        &self,
        content: &str,
        ctx: &closeclaw_common::slash_router::SlashContext,
    ) -> Option<closeclaw_common::slash_router::SlashResult> {
        Some(self.0.dispatch(content, ctx).await)
    }

    fn is_immediate(&self, command: &str) -> bool {
        self.0.is_immediate(command)
    }

    fn get_handler(
        &self,
        command: &str,
    ) -> Option<Box<dyn closeclaw_common::slash_router::SlashHandler>> {
        self.0.get_handler(command).map(|h| {
            Box::new(SlashHandlerBox { inner: h })
                as Box<dyn closeclaw_common::slash_router::SlashHandler>
        })
    }
}
