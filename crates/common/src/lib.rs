pub mod bootstrap;
pub mod code_block;
pub mod compaction;
pub mod fragment;
pub mod identity;
pub mod im_plugin;
#[cfg(test)]
pub mod im_plugin_tests;
pub mod llm_caller;
pub mod llm_error;
pub mod llm_types;
pub mod middleware;
pub mod plan_state;
#[cfg(test)]
pub mod plan_state_tests;
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
pub mod test_helpers;
pub mod tool_registry;
pub mod tool_session;
#[cfg(test)]
pub mod tool_session_tests;
pub mod tool_trait;
#[cfg(test)]
pub mod tool_trait_tests;
pub mod verbosity;

pub use bootstrap::BootstrapMode;
pub use compaction::CompactConfig;
pub use fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};
pub use identity::IdentityResolver;
pub use im_plugin::{
    AdapterError, CardActionEvent, IMAdapter, IMPlugin, MediaRef, MessageType, NormalizedMessage,
    RenderedOutput, StreamingOutput,
};
pub use llm_caller::LlmCaller;
pub use llm_error::{ErrorKind, LLMError};
pub use llm_types::{InternalMessage, InternalRequest, SystemBlock, ToolDefinition};
pub use middleware::{MiddlewareError, OutboundMiddleware};
pub use plan_state::{PlanPhase, PlanState};
pub use processor::{
    ContentBlock, ContentBlockType, ContentDelta, DslInstruction, DslParseResult, ProcessError,
    ProcessedMessage, ProcessorChain, StreamEvent, UnifiedResponse, UnifiedUsage,
};
pub use session_lookup::{PendingMessage, SessionLookup};
pub use session_types::{AgentRole, ReasoningLevel};
pub use shutdown::{DrainStatus, ShutdownMode, ShutdownSignal, ShutdownState};
pub use skill_registry::SkillRegistryQuery;
pub use slash_router::{
    SlashContext, SlashDispatcherTrait, SlashHandler, SlashResult, SlashRouter, SystemAppendAction,
};

pub use storage_provider::{PersistResult, SessionCheckpoint, SessionStatus, StorageProvider};
pub use system_prompt::{PromptOverrides, SystemPromptBuilder};
// TaskManager, TaskState, BackgroundTask, BackgroundTaskError migrated to closeclaw-tasks
pub use tool_registry::{
    RegistryError, ToolDescriptor, ToolRegistrar, ToolRegistrarError, ToolRegistry,
    ToolRegistryQuery,
};
pub use tool_session::{KillHandle, ToolSession};
pub use tool_trait::{
    build_git_status_for, build_workdir_context, ContextModifier, PromptGenerationContext, Tool,
    ToolCallError, ToolContext, ToolFlags, ToolMessage, ToolResult, WorkdirContext,
};
pub use verbosity::VerbosityLevel;
