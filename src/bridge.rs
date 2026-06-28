//! Bridge implementations — adapts main-crate concrete types to
//! `closeclaw_common` trait objects used by the gateway.

use std::sync::Arc;

use async_trait::async_trait;

use crate::daemon::shutdown::ShutdownHandle as DaemonShutdownHandle;
use crate::processor_chain::{
    context::{ProcessedMessage as MainProcessedMessage, RawMessage as MainRawMessage},
    error::ProcessError as MainProcessError,
};
use crate::slash::{
    context::SlashContext as MainSlashContext,
    handler::{SlashHandler as MainSlashHandler, SlashResult as MainSlashResult},
    SlashDispatcher,
};

// ═══════════════════════════════════════════════════════════════════════════
// ProcessorChain
// ═══════════════════════════════════════════════════════════════════════════

fn convert_processed_message(
    m: MainProcessedMessage,
) -> closeclaw_common::processor::ProcessedMessage {
    closeclaw_common::processor::ProcessedMessage {
        content: m.content,
        metadata: m.metadata,
        suppress: m.suppress,
        content_blocks: m.content_blocks,
    }
}

fn convert_process_error(e: MainProcessError) -> closeclaw_common::processor::ProcessError {
    match e {
        MainProcessError::ProcessorFailed { name, source } => {
            closeclaw_common::processor::ProcessError::ProcessorFailed { name, source }
        }
        MainProcessError::InvalidMessage(s) => {
            closeclaw_common::processor::ProcessError::InvalidMessage(s)
        }
        MainProcessError::ChainFailed(s) => {
            closeclaw_common::processor::ProcessError::ChainFailed(s)
        }
    }
}

#[async_trait]
impl closeclaw_common::processor::ProcessorChain
    for crate::processor_chain::registry::ProcessorRegistry
{
    async fn process_inbound(
        &self,
        raw: closeclaw_common::processor::RawMessage,
    ) -> Result<
        closeclaw_common::processor::ProcessedMessage,
        closeclaw_common::processor::ProcessError,
    > {
        let main_raw = MainRawMessage {
            platform: raw.platform,
            sender_id: raw.sender_id,
            peer_id: raw.peer_id,
            content: raw.content,
            timestamp: raw.timestamp,
            message_id: raw.message_id,
            account_id: raw.account_id,
        };
        self.process_inbound(main_raw)
            .await
            .map(convert_processed_message)
            .map_err(convert_process_error)
    }

    async fn process_outbound(
        &self,
        msg: closeclaw_common::processor::ProcessedMessage,
    ) -> Result<
        closeclaw_common::processor::ProcessedMessage,
        closeclaw_common::processor::ProcessError,
    > {
        let main_msg = MainProcessedMessage {
            content: msg.content,
            metadata: msg.metadata,
            suppress: msg.suppress,
            content_blocks: msg.content_blocks,
        };
        self.process_outbound(main_msg)
            .await
            .map(convert_processed_message)
            .map_err(convert_process_error)
    }

    fn inbound_len(&self) -> usize {
        self.inbound_len()
    }

    fn outbound_len(&self) -> usize {
        self.outbound_len()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SlashRouter
// ═══════════════════════════════════════════════════════════════════════════

fn convert_slash_context(ctx: &closeclaw_common::slash_router::SlashContext) -> MainSlashContext {
    MainSlashContext {
        command: ctx.command.clone(),
        sender_id: ctx.sender_id.clone(),
        session_id: ctx.session_id.clone(),
        channel: ctx.channel.clone(),
    }
}

fn convert_slash_result(result: MainSlashResult) -> closeclaw_common::slash_router::SlashResult {
    match result {
        MainSlashResult::Reply(s) => closeclaw_common::slash_router::SlashResult::Reply(s),
        MainSlashResult::SetMode(s) => closeclaw_common::slash_router::SlashResult::SetMode(s),
        MainSlashResult::NewSession => closeclaw_common::slash_router::SlashResult::NewSession,
        MainSlashResult::Stop => closeclaw_common::slash_router::SlashResult::Stop,
        MainSlashResult::Compact { instruction } => {
            closeclaw_common::slash_router::SlashResult::Compact { instruction }
        }
        MainSlashResult::SystemAppend { action } => {
            let common_action = match action {
                crate::slash::handler::SystemAppendAction::Add(s) => {
                    closeclaw_common::slash_router::SystemAppendAction::Add(s)
                }
                crate::slash::handler::SystemAppendAction::Clear => {
                    closeclaw_common::slash_router::SystemAppendAction::Clear
                }
            };
            closeclaw_common::slash_router::SlashResult::SystemAppend {
                action: common_action,
            }
        }
        MainSlashResult::Exec { command } => {
            closeclaw_common::slash_router::SlashResult::Exec { command }
        }
        MainSlashResult::SetReasoning { level } => {
            closeclaw_common::slash_router::SlashResult::SetReasoning { level }
        }
        MainSlashResult::SetVerbosity { level } => {
            closeclaw_common::slash_router::SlashResult::SetVerbosity { level }
        }
        MainSlashResult::Unknown(s) => closeclaw_common::slash_router::SlashResult::Unknown(s),
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
        let result = self.inner.handle(args, &main_ctx).await;
        convert_slash_result(result)
    }
}

#[async_trait]
impl closeclaw_common::slash_router::SlashRouter for SlashDispatcher {
    async fn dispatch(
        &self,
        content: &str,
        ctx: &closeclaw_common::slash_router::SlashContext,
    ) -> Option<closeclaw_common::slash_router::SlashResult> {
        let main_ctx = convert_slash_context(ctx);
        let result = self.dispatch(content, &main_ctx).await;
        Some(convert_slash_result(result))
    }

    fn is_immediate(&self, command: &str) -> bool {
        self.is_immediate(command)
    }

    fn get_handler(
        &self,
        command: &str,
    ) -> Option<Box<dyn closeclaw_common::slash_router::SlashHandler>> {
        self.get_handler(command).map(|h| {
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

fn convert_main_to_common_normalized(
    m: closeclaw_im_adapter::normalized::NormalizedMessage,
) -> closeclaw_common::im_plugin::NormalizedMessage {
    closeclaw_common::im_plugin::NormalizedMessage {
        platform: m.platform,
        sender_id: m.sender_id,
        peer_id: m.peer_id,
        content: m.content,
        timestamp: m.timestamp,
        message_type: m.message_type,
        media_refs: m
            .media_refs
            .into_iter()
            .map(|r| closeclaw_common::im_plugin::MediaRef {
                key: r.key,
                url: r.url,
            })
            .collect(),
        quoted_message: m
            .quoted_message
            .map(|q| closeclaw_common::im_plugin::QuotedMessage {
                content: q.content,
                sender_id: q.sender_id,
            }),
        thread_id: m.thread_id,
        account_id: m.account_id,
        card_action: m.card_action,
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
            .map(|opt| opt.map(convert_main_to_common_normalized))
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
