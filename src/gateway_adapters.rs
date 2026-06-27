//! Adapter types that bridge main-crate concrete types to closeclaw_common traits.
//!
//! The gateway crate depends on trait abstractions defined in `closeclaw_common`.
//! This module provides implementations of those traits for the main crate's
//! concrete types (ProcessorRegistry, SlashDispatcher, ToolRegistry, etc.)

use std::sync::Arc;

use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::processor::{
    ContentBlock, DslParseResult, ProcessError, ProcessedMessage, RawMessage,
};
use closeclaw_common::slash_router::{SlashHandler, SlashRouter};
use closeclaw_common::tool_registry::{ToolDescriptor, ToolRegistryQuery};

use crate::processor_chain::registry::ProcessorRegistry;

// ---------------------------------------------------------------------------
// ProcessorChain impl for ProcessorRegistry
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl closeclaw_common::processor::ProcessorChain for ProcessorRegistry {
    async fn process_inbound(&self, raw: RawMessage) -> Result<ProcessedMessage, ProcessError> {
        // Convert from closeclaw_common::processor::RawMessage to our local RawMessage
        let local_raw = crate::processor_chain::context::RawMessage {
            platform: raw.platform,
            sender_id: raw.sender_id,
            peer_id: raw.peer_id,
            content: raw.content,
            timestamp: raw.timestamp,
            message_id: raw.message_id,
            account_id: raw.account_id,
        };
        let result = ProcessorRegistry::process_inbound(self, local_raw).await?;
        Ok(ProcessedMessage {
            content: result.content,
            metadata: result.metadata,
            suppress: result.suppress,
            content_blocks: result.content_blocks,
        })
    }

    async fn process_outbound(
        &self,
        msg: ProcessedMessage,
    ) -> Result<ProcessedMessage, ProcessError> {
        let local_msg = crate::processor_chain::context::ProcessedMessage {
            content: msg.content,
            metadata: msg.metadata,
            suppress: msg.suppress,
            content_blocks: msg.content_blocks,
        };
        let result = ProcessorRegistry::process_outbound(self, local_msg).await?;
        Ok(ProcessedMessage {
            content: result.content,
            metadata: result.metadata,
            suppress: result.suppress,
            content_blocks: result.content_blocks,
        })
    }

    fn inbound_len(&self) -> usize {
        ProcessorRegistry::inbound_len(self)
    }

    fn outbound_len(&self) -> usize {
        ProcessorRegistry::outbound_len(self)
    }
}

// ---------------------------------------------------------------------------
// SlashRouter impl for SlashDispatcher
// ---------------------------------------------------------------------------

/// Wrapper that adapts the main crate's `SlashDispatcher` to the
/// `closeclaw_common::slash_router::SlashRouter` trait.
pub struct SlashRouterAdapter {
    inner: crate::slash::dispatcher::SlashDispatcher,
}

impl SlashRouterAdapter {
    pub fn new(inner: crate::slash::dispatcher::SlashDispatcher) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl SlashRouter for SlashRouterAdapter {
    async fn dispatch(
        &self,
        content: &str,
        ctx: &closeclaw_common::slash_router::SlashContext,
    ) -> Option<closeclaw_common::slash_router::SlashResult> {
        // Convert common SlashContext to main crate SlashContext
        let local_ctx = crate::slash::context::SlashContext {
            command: ctx.command.clone(),
            sender_id: ctx.sender_id.clone(),
            session_id: ctx.session_id.clone(),
            channel: ctx.channel.clone(),
        };
        let result = self.inner.dispatch(content, &local_ctx).await;
        Some(convert_slash_result(result))
    }

    fn is_immediate(&self, command: &str) -> bool {
        self.inner.is_immediate(command)
    }

    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        // SlashDispatcher.get_handler returns Option<Arc<dyn crate::slash::handler::SlashHandler>>
        // We need to wrap it in a Box<dyn closeclaw_common::slash_router::SlashHandler>
        let handler = self.inner.get_handler(command)?;
        Some(Box::new(SlashHandlerAdapter(handler)))
    }
}

/// Wrapper that adapts the main crate's `SlashHandler` to the
/// `closeclaw_common::slash_router::SlashHandler` trait.
struct SlashHandlerAdapter(Arc<dyn crate::slash::handler::SlashHandler>);

#[async_trait::async_trait]
impl SlashHandler for SlashHandlerAdapter {
    fn commands(&self) -> &[&str] {
        self.0.commands()
    }

    fn description(&self) -> &str {
        self.0.description()
    }

    fn immediate(&self, cmd: &str) -> bool {
        self.0.immediate(cmd)
    }

    fn requires_permission(&self) -> bool {
        self.0.requires_permission()
    }

    async fn handle(
        &self,
        args: &str,
        ctx: &closeclaw_common::slash_router::SlashContext,
    ) -> closeclaw_common::slash_router::SlashResult {
        let local_ctx = crate::slash::context::SlashContext {
            command: ctx.command.clone(),
            sender_id: ctx.sender_id.clone(),
            session_id: ctx.session_id.clone(),
            channel: ctx.channel.clone(),
        };
        let result = self.0.handle(args, &local_ctx).await;
        convert_slash_result(result)
    }
}

/// Convert main crate `SlashResult` to common crate `SlashResult`.
fn convert_slash_result(
    r: crate::slash::handler::SlashResult,
) -> closeclaw_common::slash_router::SlashResult {
    use crate::slash::handler::SlashResult as Src;
    use closeclaw_common::slash_router::SlashResult as Dst;
    match r {
        Src::Reply(text) => Dst::Reply(text),
        Src::SetMode(mode) => Dst::SetMode(mode),
        Src::NewSession => Dst::NewSession,
        Src::Stop => Dst::Stop,
        Src::Compact { instruction } => Dst::Compact { instruction },
        Src::SystemAppend { action } => Dst::SystemAppend {
            action: convert_system_append_action(action),
        },
        Src::Exec { command } => Dst::Exec { command },
        Src::SetReasoning { level } => Dst::SetReasoning {
            level: level.into(),
        },
        Src::SetVerbosity { level } => Dst::SetVerbosity {
            level: level.into(),
        },
        Src::Unknown(cmd) => Dst::Unknown(cmd),
    }
}

fn convert_system_append_action(
    a: crate::slash::handler::SystemAppendAction,
) -> closeclaw_common::slash_router::SystemAppendAction {
    match a {
        crate::slash::handler::SystemAppendAction::Add(s) => {
            closeclaw_common::slash_router::SystemAppendAction::Add(s)
        }
        crate::slash::handler::SystemAppendAction::Clear => {
            closeclaw_common::slash_router::SystemAppendAction::Clear
        }
    }
}

// ---------------------------------------------------------------------------
// ToolRegistryQuery impl for ToolRegistry
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl ToolRegistryQuery for crate::tools::registry::ToolRegistry {
    async fn list_tool_names(&self) -> Vec<String> {
        let guard = self.tools.read().await;
        guard.keys().cloned().collect()
    }

    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        _agent_tools: Option<&[String]>,
        _agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<ToolDescriptor> {
        let guard = self.tools.read().await;
        guard
            .values()
            .map(|t| ToolDescriptor {
                name: t.name().to_string(),
                group: t.group().to_string(),
                detail: t.summary(),
                input_schema: serde_json::Value::Object(serde_json::Map::new()),
                flags: closeclaw_common::tool_registry::ToolFlags {
                    is_deferred_by_default: t.flags().is_deferred_by_default,
                    ..Default::default()
                },
            })
            .collect()
    }

    async fn has_tool(&self, name: &str) -> bool {
        let guard = self.tools.read().await;
        guard.contains_key(name)
    }

    async fn get_tool_schema(&self, _name: &str) -> Option<serde_json::Value> {
        // Tool schema not directly accessible from the trait interface;
        // return empty schema as fallback.
        None
    }
}

// ---------------------------------------------------------------------------
// SkillRegistryQuery impl for RwLock<Option<DiskSkillRegistry>>
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl closeclaw_common::skill_registry::SkillRegistryQuery
    for std::sync::RwLock<Option<crate::skills::disk::registry::DiskSkillRegistry>>
{
    async fn has_skill(&self, name: &str) -> bool {
        let guard = self.read().expect("skill registry lock poisoned");
        guard.as_ref().map_or(false, |r| r.contains(name))
    }

    async fn list_skills(&self) -> Vec<String> {
        let guard = self.read().expect("skill registry lock poisoned");
        guard
            .as_ref()
            .map(|r| r.list().into_iter().map(String::from).collect())
            .unwrap_or_default()
    }

    async fn list_skills_for_agent(&self, agent_skills: Option<&[String]>) -> Vec<String> {
        let guard = self.read().expect("skill registry lock poisoned");
        let Some(registry) = guard.as_ref() else {
            return vec![];
        };
        let all = registry.list();
        match agent_skills {
            Some(skills) if skills.is_empty() || (skills.len() == 1 && skills[0] == "*") => {
                all.into_iter().map(String::from).collect()
            }
            Some(skills) => all
                .into_iter()
                .filter(|s| skills.iter().any(|a| a == s))
                .map(String::from)
                .collect(),
            None => all.into_iter().map(String::from).collect(),
        }
    }

    fn generate_listing(&self, agent_id: Option<&str>, agent_skills: Option<&[String]>) -> String {
        let guard = self.read().expect("skill registry lock poisoned");
        match (&guard, agent_id) {
            (Some(registry), Some(aid)) => {
                // Use agent-specific listing if available
                let all = registry.list();
                match agent_skills {
                    Some(skills)
                        if skills.is_empty() || (skills.len() == 1 && skills[0] == "*") =>
                    {
                        registry.generate_listing_for_agent(aid)
                    }
                    Some(skills) => {
                        let filtered: Vec<&str> = all
                            .into_iter()
                            .filter(|s| skills.iter().any(|a| a == *s))
                            .collect();
                        if filtered.is_empty() {
                            String::new()
                        } else {
                            filtered
                                .iter()
                                .map(|s| format!("- {}", s))
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    }
                    None => registry.generate_listing_for_agent(aid),
                }
            }
            (Some(registry), None) => {
                let all = registry.list();
                all.iter()
                    .map(|s| format!("- {}", s))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            (None, _) => String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// IMPlugin adapter for concrete plugins
// ---------------------------------------------------------------------------

/// Wrapper that adapts the main crate's `IMPlugin` to the
/// `closeclaw_common::IMPlugin` trait.
pub struct IMPluginAdapter {
    inner: Arc<dyn crate::im_adapter::plugin::IMPlugin>,
}

impl IMPluginAdapter {
    pub fn new(inner: Arc<dyn crate::im_adapter::plugin::IMPlugin>) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl closeclaw_common::IMPlugin for IMPluginAdapter {
    fn platform(&self) -> &str {
        self.inner.platform()
    }

    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<closeclaw_common::NormalizedMessage>, closeclaw_common::AdapterError> {
        self.inner
            .parse_inbound(payload)
            .await
            .map(|opt| opt.map(convert_normalized_message))
            .map_err(convert_adapter_error)
    }

    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool {
        self.inner.validate_signature(signature, payload).await
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), closeclaw_common::AdapterError> {
        let local_output = crate::im_adapter::plugin::RenderedOutput {
            msg_type: output.msg_type.clone(),
            payload: output.payload.clone(),
        };
        self.inner
            .send(&local_output, peer_id, thread_id)
            .await
            .map_err(convert_adapter_error)
    }

    fn clean_content(&self, raw: &str) -> String {
        self.inner.clean_content(raw)
    }

    async fn init(&self) -> Result<(), closeclaw_common::AdapterError> {
        self.inner.init().await.map_err(convert_adapter_error)
    }

    async fn shutdown(&self) -> Result<(), closeclaw_common::AdapterError> {
        self.inner.shutdown().await.map_err(convert_adapter_error)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let local_dsl = dsl_result.map(convert_dsl_parse_result);
        let result = self.inner.render(content_blocks, local_dsl.as_ref());
        RenderedOutput {
            msg_type: result.msg_type,
            payload: result.payload,
        }
    }
}

fn convert_normalized_message(
    m: crate::im_adapter::plugin::NormalizedMessage,
) -> closeclaw_common::NormalizedMessage {
    closeclaw_common::NormalizedMessage {
        id: m.id,
        sender_id: m.sender_id,
        peer_id: m.peer_id,
        content: m.content,
        timestamp: m.timestamp,
        message_type: m.message_type,
        account_id: m.account_id,
        thread_id: m.thread_id,
        media_refs: m.media_refs.into_iter().map(convert_media_ref).collect(),
        quoted: m.quoted.map(convert_quoted_message),
    }
}

fn convert_media_ref(m: crate::im_adapter::plugin::MediaRef) -> closeclaw_common::MediaRef {
    closeclaw_common::MediaRef {
        key: m.key,
        url: m.url,
    }
}

fn convert_quoted_message(
    q: crate::im_adapter::plugin::QuotedMessage,
) -> closeclaw_common::QuotedMessage {
    closeclaw_common::QuotedMessage {
        content: q.content,
        sender_id: q.sender_id,
    }
}

fn convert_adapter_error(
    e: crate::im_adapter::error::AdapterError,
) -> closeclaw_common::AdapterError {
    match e {
        crate::im_adapter::error::AdapterError::InvalidPayload(s) => {
            closeclaw_common::AdapterError::InvalidPayload(s)
        }
        crate::im_adapter::error::AdapterError::AuthFailed => {
            closeclaw_common::AdapterError::AuthFailed
        }
        crate::im_adapter::error::AdapterError::SendFailed(s) => {
            closeclaw_common::AdapterError::SendFailed(s)
        }
        crate::im_adapter::error::AdapterError::InvalidSignature => {
            closeclaw_common::AdapterError::InvalidSignature
        }
        crate::im_adapter::error::AdapterError::IoError(e) => {
            closeclaw_common::AdapterError::IoError(e)
        }
        crate::im_adapter::error::AdapterError::UnsupportedOperation => {
            closeclaw_common::AdapterError::UnsupportedOperation
        }
    }
}

fn convert_dsl_parse_result(
    d: &crate::processor_chain::dsl_parser::DslParseResult,
) -> DslParseResult {
    DslParseResult {
        clean_content: d.clean_content.clone(),
        instructions: d.instructions.iter().map(convert_dsl_instruction).collect(),
    }
}

fn convert_dsl_instruction(
    i: &crate::processor_chain::dsl_parser::DslInstruction,
) -> closeclaw_common::DslInstruction {
    match i {
        crate::processor_chain::dsl_parser::DslInstruction::Button {
            label,
            action,
            value,
        } => closeclaw_common::DslInstruction::Button {
            label: label.clone(),
            action: action.clone(),
            value: value.clone(),
        },
        crate::processor_chain::dsl_parser::DslInstruction::Selector {
            label,
            options,
            action,
        } => closeclaw_common::DslInstruction::Selector {
            label: label.clone(),
            options: options.clone(),
            action: action.clone(),
        },
    }
}
