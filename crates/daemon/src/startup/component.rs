//! Component identity types and per-component startup dependency declarations.
//!
//! Re-exported through the `startup` module; the topological sorter that
//! consumes these declarations lives in `super::graph`.

/// Core infrastructure components with no dependencies (Layer 0/1).
///
/// Separated from [`Service`] to keep the overall enum variant count
/// within the 20-variant CI limit while supporting additional service
/// components like [`Service::LLMRegistry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Foundation {
    /// Loads and merges configuration files.
    ConfigManager,
    /// SQLite-backed session persistence.
    Storage,
}

/// Service-layer components that depend on [`Foundation`] components.
///
/// Separated from [`Foundation`] to keep the overall enum variant count
/// within the 20-variant CI limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    /// Per-agent idle/purge thresholds from session_config.json.
    SessionConfigProvider,
    /// Agent configuration registry.
    AgentRegistry,
    /// Scanned and registered skill definitions.
    SkillsRegistry,
    /// Platform-specific renderers and plugins.
    RenderersPlugins,
    /// Platform-specific IM adapters.
    IMAdapters,
    /// Global and per-agent permission rules.
    PermissionEngine,
    /// Tool definitions from all modules.
    ToolsRegistry,
    /// Background idle session archiver.
    ArchiveSweeper,
    /// Background announce delivery sweeper for spawn silent-failure protection.
    AnnounceSweeper,
    /// Background config file hot-reload watcher.
    ConfigHotReload,
    /// Background dreaming/memory-mining scheduler.
    DreamingScheduler,
    /// Session lifecycle manager.
    SessionManager,
    /// System prompt builder.
    SystemPromptBuilder,
    /// High-risk slash command approval orchestrator.
    ApprovalFlow,
    /// Top-level message router.
    Gateway,
    /// Validates Agent spawn permissions, injected into ToolRegistry.
    SpawnController,
    /// Unix domain socket management service for CLI Admin commands.
    AdminRpcServer,
    /// LLM provider registry — reads models.json, constructs LLM clients.
    LLMRegistry,
    /// Background plan archive sweeper — archives old plan files.
    PlanArchiveSweeper,
}

/// Identifies a daemon component for startup orchestration.
///
/// Wraps [`Foundation`] and [`Service`] sub-enums. The `name()` method
/// provides a stable, human-readable label used for alphabetical ordering
/// within each layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentId {
    /// Core infrastructure with no dependencies.
    Foundation(Foundation),
    /// Service-layer components with foundation dependencies.
    Service(Service),
}

impl ComponentId {
    /// Stable display name for this component.
    ///
    /// Used as the sort key for deterministic layer-internal ordering.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Foundation(f) => match f {
                Foundation::ConfigManager => "ConfigManager",
                Foundation::Storage => "Storage",
            },
            Self::Service(s) => match s {
                Service::SessionConfigProvider => "SessionConfigProvider",
                Service::AgentRegistry => "AgentRegistry",
                Service::SkillsRegistry => "SkillsRegistry",
                Service::RenderersPlugins => "RenderersPlugins",
                Service::IMAdapters => "IMAdapters",
                Service::PermissionEngine => "PermissionEngine",
                Service::ToolsRegistry => "ToolsRegistry",
                Service::ArchiveSweeper => "ArchiveSweeper",
                Service::AnnounceSweeper => "AnnounceSweeper",
                Service::ConfigHotReload => "ConfigHotReload",
                Service::DreamingScheduler => "DreamingScheduler",
                Service::SessionManager => "SessionManager",
                Service::SystemPromptBuilder => "SystemPromptBuilder",
                Service::ApprovalFlow => "ApprovalFlow",
                Service::Gateway => "Gateway",
                Service::SpawnController => "SpawnController",
                Service::AdminRpcServer => "AdminRpcServer",
                Service::LLMRegistry => "LLMRegistry",
                Service::PlanArchiveSweeper => "PlanArchiveSweeper",
            },
        }
    }
}

/// Declares the startup dependencies of a daemon component.
///
/// Implementations return the set of [`ComponentId`]s that must be fully
/// initialized before this component can start.
pub trait ComponentDeps {
    /// Returns the component IDs that this component depends on.
    fn deps(&self) -> &[ComponentId];
}

/// A component entry fed into the topological sorter.
///
/// Bundles the component identity, its human-readable name (for alphabetical
/// sorting), and its declared dependencies into a single value.
pub struct ComponentEntry {
    /// The component identifier.
    pub id: ComponentId,
    /// Human-readable name, used as the sort key within a layer.
    pub name: &'static str,
    /// IDs of components that must be initialized before this one.
    pub deps: Vec<ComponentId>,
}

impl ComponentDeps for ComponentId {
    fn deps(&self) -> &[ComponentId] {
        use self::Foundation::*;
        use self::Service::*;
        match self {
            Self::Foundation(ConfigManager) => &[],
            Self::Foundation(Storage) => &[],
            Self::Service(SessionConfigProvider) => &[Self::Foundation(ConfigManager)],
            Self::Service(AgentRegistry) => &[Self::Foundation(ConfigManager)],
            Self::Service(SkillsRegistry) => &[Self::Foundation(ConfigManager)],
            Self::Service(RenderersPlugins) => &[Self::Foundation(ConfigManager)],
            Self::Service(IMAdapters) => &[
                Self::Service(RenderersPlugins),
                Self::Foundation(ConfigManager),
            ],
            Self::Service(PermissionEngine) => &[Self::Foundation(ConfigManager)],
            Self::Service(ToolsRegistry) => &[Self::Service(SkillsRegistry)],
            Self::Service(ArchiveSweeper) => &[
                Self::Foundation(Storage),
                Self::Service(SessionConfigProvider),
            ],
            Self::Service(AnnounceSweeper) => &[
                Self::Foundation(Storage),
                Self::Service(SessionConfigProvider),
            ],
            Self::Service(ConfigHotReload) => &[Self::Foundation(ConfigManager)],
            Self::Service(DreamingScheduler) => &[
                Self::Foundation(Storage),
                Self::Service(SessionConfigProvider),
            ],
            Self::Service(LLMRegistry) => &[Self::Foundation(ConfigManager)],
            Self::Service(SessionManager) => &[
                Self::Service(LLMRegistry),
                Self::Foundation(Storage),
                Self::Service(AgentRegistry),
                Self::Service(SkillsRegistry),
                Self::Service(ToolsRegistry),
                Self::Service(SessionConfigProvider),
            ],
            Self::Service(SystemPromptBuilder) => &[
                Self::Service(AgentRegistry),
                Self::Service(SkillsRegistry),
                Self::Service(ToolsRegistry),
            ],
            Self::Service(ApprovalFlow) => &[
                Self::Service(PermissionEngine),
                Self::Service(AgentRegistry),
            ],
            Self::Service(Gateway) => &[
                Self::Service(SessionManager),
                Self::Service(IMAdapters),
                Self::Service(PermissionEngine),
                Self::Service(ApprovalFlow),
                Self::Service(RenderersPlugins),
            ],
            Self::Service(SpawnController) => {
                &[Self::Service(AgentRegistry), Self::Service(ToolsRegistry)]
            }
            Self::Service(PlanArchiveSweeper) => &[Self::Foundation(ConfigManager)],
            Self::Service(AdminRpcServer) => &[Self::Service(Gateway)],
        }
    }
}
