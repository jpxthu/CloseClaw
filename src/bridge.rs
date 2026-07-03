//! Bridge implementations — adapts main-crate concrete types to
//! `closeclaw_common` trait objects used by the gateway.

use std::sync::Arc;

use async_trait::async_trait;

use crate::slash::{
    context::SlashContext as MainSlashContext, handler::SlashHandler as MainSlashHandler,
    SlashDispatcher,
};
use closeclaw_daemon::shutdown::ShutdownHandle as DaemonShutdownHandle;

/// Newtype wrapper around `SlashDispatcher` to satisfy the orphan rule
/// when implementing `closeclaw_common::slash_router::SlashRouter`.
pub struct SlashDispatcherWrapper(pub SlashDispatcher);

// ═══════════════════════════════════════════════════════════════════════════
// ProcessorChain
// ═══════════════════════════════════════════════════════════════════════════

fn convert_slash_context(ctx: &closeclaw_common::slash_router::SlashContext) -> MainSlashContext {
    MainSlashContext {
        command: ctx.command.clone(),
        sender_id: ctx.sender_id.clone(),
        session_id: ctx.session_id.clone(),
        channel: ctx.channel.clone(),
    }
}

/// Adapter wrapping a main-crate `SlashHandler` to implement the common `SlashHandler` trait.
struct SlashHandlerAdapter {
    inner: Arc<dyn MainSlashHandler>,
}

#[async_trait]
impl closeclaw_common::slash_router::SlashHandler for SlashHandlerAdapter {
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
        let main_ctx = convert_slash_context(ctx);
        self.inner.handle(args, &main_ctx).await
    }
}

#[async_trait]
impl closeclaw_common::slash_router::SlashRouter for SlashDispatcherWrapper {
    async fn dispatch(
        &self,
        content: &str,
        ctx: &closeclaw_common::slash_router::SlashContext,
    ) -> Option<closeclaw_common::slash_router::SlashResult> {
        let main_ctx = convert_slash_context(ctx);
        Some(self.0.dispatch(content, &main_ctx).await)
    }

    fn is_immediate(&self, command: &str) -> bool {
        self.0.is_immediate(command)
    }

    fn get_handler(
        &self,
        command: &str,
    ) -> Option<Box<dyn closeclaw_common::slash_router::SlashHandler>> {
        self.0.get_handler(command).map(|h| {
            Box::new(SlashHandlerAdapter { inner: h })
                as Box<dyn closeclaw_common::slash_router::SlashHandler>
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SkillRegistryQuery — newtype wrapper (orphan rule)
// ═══════════════════════════════════════════════════════════════════════════

/// Newtype wrapper around `Arc<RwLock<Option<DiskSkillRegistry>>>` to
/// satisfy the orphan rule when implementing `SkillRegistryQuery`.
pub struct SkillRegistryWrapper(
    pub Arc<std::sync::RwLock<Option<closeclaw_skills::DiskSkillRegistry>>>,
);

#[async_trait]
impl closeclaw_common::skill_registry::SkillRegistryQuery for SkillRegistryWrapper {
    async fn has_skill(&self, name: &str) -> bool {
        self.0
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|r| r.contains(name)))
            .unwrap_or(false)
    }

    async fn list_skills(&self) -> Vec<String> {
        self.0
            .read()
            .ok()
            .and_then(|g| {
                g.as_ref()
                    .map(|r| r.list().into_iter().map(String::from).collect())
            })
            .unwrap_or_default()
    }

    async fn list_skills_for_agent(&self, agent_skills: Option<&[String]>) -> Vec<String> {
        self.0
            .read()
            .ok()
            .and_then(|g| {
                g.as_ref().map(|r| {
                    let all = r.list();
                    match agent_skills {
                        Some(skills) if skills.len() == 1 && skills[0] == "*" => {
                            all.into_iter().map(String::from).collect()
                        }
                        Some([]) => all.into_iter().map(String::from).collect(),
                        Some(skills) => {
                            let set: std::collections::HashSet<&str> =
                                skills.iter().map(|s| s.as_str()).collect();
                            all.into_iter()
                                .filter(|name| set.contains(*name))
                                .map(String::from)
                                .collect()
                        }
                        None => all.into_iter().map(String::from).collect(),
                    }
                })
            })
            .unwrap_or_default()
    }

    fn generate_listing(&self, agent_id: Option<&str>, agent_skills: Option<&[String]>) -> String {
        self.0
            .read()
            .ok()
            .and_then(|g| {
                g.as_ref()
                    .map(|r| r.generate_listing(agent_id, agent_skills))
            })
            .unwrap_or_default()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ShutdownHandle conversion
// ═══════════════════════════════════════════════════════════════════════════

// DaemonShutdownMode is now a re-export of closeclaw_common::ShutdownMode,
// so no conversion is needed.

/// Create a `closeclaw_common::shutdown::ShutdownHandle` from the daemon's
/// `ShutdownHandle`. The common handle wraps the daemon's handle as a
/// `dyn ShutdownSignal`.
pub fn common_shutdown_handle(
    daemon_handle: &DaemonShutdownHandle,
) -> Arc<closeclaw_common::shutdown::ShutdownHandle> {
    Arc::new(closeclaw_common::shutdown::ShutdownHandle::new(Arc::new(
        daemon_handle.clone(),
    )))
}

// ═══════════════════════════════════════════════════════════════════════════
// IMPlugin adapter
// ═══════════════════════════════════════════════════════════════════════════

/// Adapter that wraps a main-crate `IMPlugin` and implements the common
/// `IMPlugin` trait for gateway registration.
pub struct IMPluginAdapter {
    inner: Arc<dyn closeclaw_im_adapter::plugin::IMPlugin>,
}

impl IMPluginAdapter {
    /// Wrap a main-crate IMPlugin for use with the gateway.
    pub fn wrap(
        plugin: Arc<dyn closeclaw_im_adapter::plugin::IMPlugin>,
    ) -> Arc<dyn closeclaw_common::IMPlugin> {
        Arc::new(Self { inner: plugin })
    }
}

fn convert_common_adapter_error(
    e: closeclaw_im_adapter::error::AdapterError,
) -> closeclaw_common::im_plugin::AdapterError {
    match e {
        closeclaw_im_adapter::error::AdapterError::InvalidPayload(s) => {
            closeclaw_common::im_plugin::AdapterError::InvalidPayload(s)
        }
        closeclaw_im_adapter::error::AdapterError::AuthFailed => {
            closeclaw_common::im_plugin::AdapterError::AuthFailed
        }
        closeclaw_im_adapter::error::AdapterError::SendFailed(s) => {
            closeclaw_common::im_plugin::AdapterError::SendFailed(s)
        }
        closeclaw_im_adapter::error::AdapterError::InvalidSignature => {
            closeclaw_common::im_plugin::AdapterError::InvalidSignature
        }
        closeclaw_im_adapter::error::AdapterError::IoError(e) => {
            closeclaw_common::im_plugin::AdapterError::IoError(e)
        }
        closeclaw_im_adapter::error::AdapterError::MediaDownloadFailed(s) => {
            closeclaw_common::im_plugin::AdapterError::SendFailed(s)
        }
        closeclaw_im_adapter::error::AdapterError::UnsupportedOperation => {
            closeclaw_common::im_plugin::AdapterError::UnsupportedOperation
        }
    }
}

fn convert_common_to_main_rendered(
    output: &closeclaw_common::im_plugin::RenderedOutput,
) -> closeclaw_im_adapter::plugin::RenderedOutput {
    closeclaw_im_adapter::plugin::RenderedOutput {
        msg_type: output.msg_type.clone(),
        payload: output.payload.clone(),
    }
}

fn convert_common_streaming_output(
    o: closeclaw_im_adapter::streaming::StreamingOutput,
) -> closeclaw_common::im_plugin::StreamingOutput {
    closeclaw_common::im_plugin::StreamingOutput {
        text_messages: o.text_messages,
        render_blocks: o.render_blocks,
    }
}

#[async_trait]
impl closeclaw_common::IMPlugin for IMPluginAdapter {
    fn platform(&self) -> &str {
        self.inner.platform()
    }

    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<
        Option<closeclaw_common::im_plugin::NormalizedMessage>,
        closeclaw_common::im_plugin::AdapterError,
    > {
        self.inner
            .parse_inbound(payload)
            .await
            .map_err(convert_common_adapter_error)
    }

    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool {
        self.inner.validate_signature(signature, payload).await
    }

    async fn send(
        &self,
        output: &closeclaw_common::im_plugin::RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        let main_output = convert_common_to_main_rendered(output);
        self.inner
            .send(&main_output, peer_id, thread_id)
            .await
            .map_err(convert_common_adapter_error)
    }

    fn clean_content(&self, raw: &str) -> String {
        self.inner.clean_content(raw)
    }

    async fn init(&self) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        self.inner
            .init()
            .await
            .map_err(convert_common_adapter_error)
    }

    async fn shutdown(&self) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        self.inner
            .shutdown()
            .await
            .map_err(convert_common_adapter_error)
    }

    async fn shutdown_inbound(&self) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        self.inner
            .shutdown_inbound()
            .await
            .map_err(convert_common_adapter_error)
    }

    async fn shutdown_outbound(&self) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        self.inner
            .shutdown_outbound()
            .await
            .map_err(convert_common_adapter_error)
    }

    fn render(
        &self,
        content_blocks: &[closeclaw_common::processor::ContentBlock],
        dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> closeclaw_common::im_plugin::RenderedOutput {
        // The new crate uses closeclaw_common types directly — no conversion needed.
        let result = self.inner.render(content_blocks, dsl_result);
        closeclaw_common::im_plugin::RenderedOutput {
            msg_type: result.msg_type,
            payload: result.payload,
        }
    }

    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        // The new crate uses closeclaw_common types directly — no conversion needed.
        let result = self.inner.handle_stream_event(event);
        convert_common_streaming_output(result)
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        convert_common_streaming_output(self.inner.flush_stream())
    }
}
