pub mod agent_config;
pub mod agent_config_lookup;
pub mod agent_lookup;
pub mod agent_skills_query;
pub mod agent_tools_config_query;
pub mod bootstrap;
pub mod compaction;
pub mod gateway_spawn;
pub mod gateway_stop;
pub mod gateway_types;
pub mod identity;
pub mod im_plugin;
#[cfg(test)]
pub mod im_plugin_tests;
pub mod middleware;
pub mod processor;
#[cfg(test)]
pub mod processor_tests;
pub mod session_lookup;
pub mod session_types;
pub mod shutdown;
pub mod skill_registry;
pub mod slash_router;
#[cfg(test)]
pub mod slash_router_tests;
pub mod storage_provider;
pub mod streaming;
pub mod system_prompt;
pub mod task_manager;
pub mod test_helpers;
pub mod tool_registry;
pub mod verbosity;

pub use agent_config_lookup::{AgentConfigInfo, AgentConfigLookup};
pub use agent_lookup::AgentLookup;
pub use agent_skills_query::AgentSkillsQuery;
pub use agent_tools_config_query::{AgentToolsConfig, AgentToolsConfigQuery};
pub use bootstrap::BootstrapMode;
pub use compaction::CompactConfig;
pub use gateway_spawn::{ChildSessionInfo, SpawnMode};
pub use gateway_stop::{StopProgress, StopResult};
pub use gateway_types::{
    DmScope, GatewayConfig, GatewayError, HandleResult, InboundChainInput, InboundRequest, Message,
    Session,
};
pub use identity::IdentityResolver;
pub use im_plugin::{
    AdapterError, CardActionEvent, IMAdapter, IMPlugin, InboundEvent, MediaRef, MessageType,
    NormalizedMessage, RenderedOutput, StreamingOutput,
};
pub use middleware::{MiddlewareError, OutboundMiddleware};
pub use processor::{
    ContentBlock, ContentBlockType, ContentDelta, DslInstruction, DslParseResult, ProcessError,
    ProcessedMessage, ProcessorChain, StreamEvent, UnifiedResponse, UnifiedUsage,
};
pub use session_lookup::{PendingMessage, SessionLookup};
pub use session_types::{AgentRole, ReasoningLevel};
pub use shutdown::{DrainStatus, ShutdownMode, ShutdownSignal, ShutdownState};
pub use skill_registry::SkillRegistryQuery;
pub use slash_router::{
    ReplyAction, SideEffectContext, SlashContext, SlashDispatcherTrait, SlashHandler, SlashResult,
    SlashRouter, SystemAppendAction,
};

pub use storage_provider::{PersistResult, SessionCheckpoint, SessionStatus, StorageProvider};
pub use system_prompt::{PromptOverrides, SystemPromptBuilder, WorkspaceBuildConfig};
pub use task_manager::{BackgroundTask, BackgroundTaskError, TaskManager, TaskState};
pub use tool_registry::{ToolDescriptor, ToolFlags, ToolRegistryQuery};
pub use verbosity::VerbosityLevel;
