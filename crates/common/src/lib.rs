pub mod agent_config;
#[cfg(test)]
pub mod agent_config_tests;
pub mod agent_lookup;
#[cfg(test)]
pub mod agent_lookup_tests;
pub mod agent_query;
pub mod audit_log;
pub mod background_task;
pub mod bootstrap;
pub mod communication;
pub mod compaction;
pub mod content_segment;
pub mod execution_types;
pub mod executor;
#[cfg(test)]
pub mod executor_test_utils;
#[cfg(test)]
pub mod executor_tests;

pub mod dispatcher;
pub mod file_mutex;
pub mod fragment;
pub mod hook_config;
pub mod identity;
pub mod im_plugin;
#[cfg(test)]
pub mod im_plugin_tests;
pub mod injection_params;
pub mod llm_caller;
pub mod llm_error;
pub mod llm_stats;
#[cfg(test)]
pub mod llm_stats_tests;
pub mod llm_streaming;
pub mod llm_types;
pub mod media_store;
pub mod memory_config;
#[cfg(test)]
pub mod memory_config_tests;
pub mod metrics;
pub mod middleware;
pub mod model_spec;
#[cfg(test)]
pub mod model_spec_tests;
pub mod path_utils;
pub mod permission_check;
pub mod permission_op;
#[cfg(test)]
pub mod permission_op_tests;
pub mod permission_types;
pub mod plan_confirm_handler;
pub mod plan_state;
#[cfg(test)]
pub mod plan_state_tests;
pub mod processor;
#[cfg(test)]
pub mod processor_tests;
pub mod request_context;
pub mod session_key;
pub mod session_lookup;
pub mod session_mode;
pub mod session_mode_query;
pub mod session_state;
pub mod session_types;
pub mod shutdown;
#[cfg(test)]
pub mod shutdown_tests;
pub mod skill_listing_provider;
pub mod skill_registry;
pub mod slash_router;
#[cfg(test)]
pub mod slash_router_tests;
pub mod slash_session_query;
pub mod spawn_validation;
#[cfg(test)]
pub mod spawn_validation_tests;
pub mod streaming;
#[cfg(test)]
pub mod streaming_tests;
pub mod system_prompt;
#[cfg(test)]
pub mod system_prompt_tests;
pub mod task_manager;
#[cfg(test)]
pub mod task_manager_tests;
pub mod test_helpers;
#[cfg(test)]
pub mod test_helpers_tests;
pub mod tool_registry;
pub mod tool_session;
#[cfg(test)]
pub mod tool_session_tests;
pub mod tool_trait;
#[cfg(test)]
pub mod tool_trait_tests;
pub mod trace_id;
pub mod turn;
pub mod verbosity;

pub use agent_config::{ConfigSource, ResolvedAgentConfig, SubagentsConfig};
pub use agent_lookup::{AgentConfigInfo, AgentConfigLookup, AgentLookup, AgentRegistryQuery};
pub use agent_query::{AgentSkillsQuery, AgentToolsConfig, AgentToolsConfigQuery};
pub use audit_log::{AuditDisposition, AuditLogEntry, AuditLogFilter, AuditLogger};
pub use background_task::{
    BackgroundTask, BackgroundTaskError, CompletionNotification, NotificationPriority,
    RunningTaskInfo, TaskState,
};
pub use bootstrap::BootstrapMode;
pub use compaction::CompactConfig;
pub use content_segment::{parse_content_segments, ContentSegment};
pub use execution_types::ExecutionStepStatus;
pub use fragment::{
    FragmentContext, PromptFragment, PromptFragmentProvider, SectionType, SessionRole,
};
pub use hook_config::{HookConfig, HookParams, HookType};
pub use identity::IdentityResolver;
pub use im_plugin::{
    AdapterError, CardActionEvent, IMPlugin, MediaRef, MediaType, MessageType, NormalizedMessage,
    RenderedOutput, StreamingOutput,
};
pub use injection_params::InjectionParams;
pub use llm_caller::LlmCaller;
pub use llm_error::{ErrorKind, LLMError};
pub use llm_stats::{detect_cache_break, CacheBreakInfo, CacheBreakThresholds, RunningStats};
pub use llm_streaming::{StreamDone, StreamingSink};
pub use llm_types::{InternalMessage, InternalRequest, SystemBlock, ToolDefinition};
pub use media_store::{MediaStoreAccess, MediaStoreError};
pub use memory_config::{
    default_capacity_max_rules, default_db_path, default_diary_path, default_dreaming_schedule,
    default_forgetting_initial_ttl_days, default_forgetting_injection_extension_days,
    default_forgetting_reidentify_extension_days, default_memory_md_path,
    default_mining_dedup_window_days, default_mining_max_events_per_session,
    default_scoring_cross_agent, default_scoring_explicitness, default_scoring_frequency,
    default_scoring_negative_signal, default_scoring_recency, default_search_context_turns,
    default_search_max_summary_chars, default_search_min_entity_hits, default_search_timeout_ms,
    default_search_top_k_events, default_threshold_absolute, default_threshold_relative,
    default_transcript_format, default_transcript_min_owner_msgs, default_transcript_min_turns,
    DreamingCapacityConfig, DreamingConfig, DreamingDiaryConfig, DreamingScoringConfig,
    DreamingThresholdConfig, ForgettingConfig, MemoryConfig, MemoryStorageConfig, MiningConfig,
    SearchConfig, TranscriptCleanRules,
};
pub use metrics::MetricsEmitter;
pub use middleware::{MiddlewareContext, MiddlewareError, OutboundMiddleware};
pub use model_spec::ModelSpec;
pub use path_utils::canonicalize_or_clone;
pub use permission_check::{PermissionChecker, PermissionDenied, SpawnPermissionError};
pub use permission_op::{InitialPermissionSet, UserCreationRequest, UserRegistration};
pub use plan_state::{PlanPhase, PlanState};
pub use processor::{
    ContentBlock, ContentBlockType, ContentDelta, DslInstruction, DslParseResult, ProcessError,
    ProcessedMessage, ProcessorChain, StreamEvent, UnifiedResponse, UnifiedUsage,
};
pub use request_context::RequestContext;
pub use session_lookup::{PendingMessage, SessionLookup};
pub use session_state::{
    ChildCompletionStatus, ChildSessionState, LlmState, SessionActivityDimensions,
    SessionExecStatus, ToolExecState,
};
pub use session_types::{AgentRole, ReasoningLevel};
pub use shutdown::{DrainStatus, ShutdownMode, ShutdownSignal, ShutdownState};
pub use skill_listing_provider::ConditionalSkillMatch;
pub use skill_listing_provider::SkillListingProvider;
pub use skill_registry::SkillRegistryQuery;
pub use slash_router::{SlashContext, SlashHandler, SlashResult, SlashRouter, SystemAppendAction};
pub use slash_session_query::SlashSessionQuery;
pub use spawn_validation::{SpawnError, SpawnValidationResult, SpawnValidator};
pub use streaming::{CodeBlockMode, DefaultStreamingRenderer, LineBuffer, StreamingRenderer};
pub use turn::TurnCounter;

pub use communication::{
    check_communication_allowed, CommunicationCheckResult, CommunicationConfig, CommunicationError,
};
// Executor types: defined here (not in slash) because gateway cannot
// depend on slash (cycle: gateway -> slash -> tools -> gateway).
pub use dispatcher::{
    extract_file_path, DispatchGroup, PendingToolCall, ToolCallDispatcher, ToolExecutor,
};
pub use executor::{
    CompactionError, CompactionResult, ReplyAction, SideEffectContext, SlashEffectExecutor,
    SlashResultExecutor,
};
pub use file_mutex::FileMutexMap;
pub use session_mode::SessionMode;
pub use session_mode_query::SessionModeQuery;
pub use system_prompt::{
    split_static_dynamic, DynamicPromptBuilder, DynamicPromptContext, ModeTransition,
    PromptOverrides, SystemPromptBuilder,
};
pub use task_manager::TaskManager;
pub use tool_registry::{
    RegistryError, ToolBox, ToolDescriptor, ToolRegistrar, ToolRegistrarError, ToolRegistry,
    ToolRegistryQuery,
};
pub use tool_session::{FileReadCache, KillHandle, ReadRange, ToolProgress, ToolSession};
pub use tool_trait::{
    build_git_status_for, build_workdir_context, format_workdir_guidance, ContextModifier,
    PromptGenerationContext, Tool, ToolCallError, ToolContext, ToolFlags, ToolMessage, ToolResult,
    WorkdirContext,
};
pub use trace_id::generate_trace_id;
pub use verbosity::VerbosityLevel;
