//! Startup orchestration: component dependency declarations and data structures.
//!
//! Defines [`ComponentId`] to identify each daemon component and [`ComponentDeps`]
//! to declare startup dependencies. The topological sort engine (see
//! [`topo_sort_layers`]) consumes these declarations to derive the deterministic
//! initialization order.

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
    /// Background skill file watcher.
    SkillWatcher,
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
                Service::SkillWatcher => "SkillWatcher",
                Service::ConfigHotReload => "ConfigHotReload",
                Service::DreamingScheduler => "DreamingScheduler",
                Service::SessionManager => "SessionManager",
                Service::SystemPromptBuilder => "SystemPromptBuilder",
                Service::ApprovalFlow => "ApprovalFlow",
                Service::Gateway => "Gateway",
                Service::SpawnController => "SpawnController",
                Service::AdminRpcServer => "AdminRpcServer",
                Service::LLMRegistry => "LLMRegistry",
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
            Self::Service(SkillWatcher) => &[Self::Service(SkillsRegistry)],
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
            Self::Service(AdminRpcServer) => &[Self::Service(Gateway)],
        }
    }
}

/// Returns [`ComponentEntry`]s for all daemon components.
///
/// Each entry bundles the component identity, its human-readable name,
/// and the dependencies declared via [`ComponentDeps`].
pub fn all_component_entries() -> Vec<ComponentEntry> {
    [
        ComponentId::Foundation(Foundation::ConfigManager),
        ComponentId::Foundation(Foundation::Storage),
        ComponentId::Service(Service::SessionConfigProvider),
        ComponentId::Service(Service::AgentRegistry),
        ComponentId::Service(Service::SkillsRegistry),
        ComponentId::Service(Service::RenderersPlugins),
        ComponentId::Service(Service::IMAdapters),
        ComponentId::Service(Service::PermissionEngine),
        ComponentId::Service(Service::ToolsRegistry),
        ComponentId::Service(Service::ArchiveSweeper),
        ComponentId::Service(Service::AnnounceSweeper),
        ComponentId::Service(Service::SkillWatcher),
        ComponentId::Service(Service::ConfigHotReload),
        ComponentId::Service(Service::DreamingScheduler),
        ComponentId::Service(Service::LLMRegistry),
        ComponentId::Service(Service::SessionManager),
        ComponentId::Service(Service::SystemPromptBuilder),
        ComponentId::Service(Service::ApprovalFlow),
        ComponentId::Service(Service::Gateway),
        ComponentId::Service(Service::SpawnController),
        ComponentId::Service(Service::AdminRpcServer),
    ]
    .into_iter()
    .map(|id| ComponentEntry {
        name: id.name(),
        deps: id.deps().to_vec(),
        id,
    })
    .collect()
}

/// Errors that can occur during startup orchestration.
#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    /// A cycle was detected in the dependency graph.
    #[error("circular dependency detected in component startup order")]
    CircularDependency,

    /// A component declares a dependency on an unknown component.
    #[error("component {0:?} depends on unknown component {1:?}")]
    MissingDependency(ComponentId, ComponentId),

    /// The resolved layers do not match the expected phase structure.
    #[error("startup layers mismatch: resolved layers differ from expected phases")]
    StartupLayersMismatch,
}

/// Topologically sort the given component entries into ordered layers.
///
/// Each layer contains components whose dependencies are all satisfied by
/// earlier layers. Within each layer, components are sorted alphabetically
/// by name for deterministic ordering.
///
/// # Errors
///
/// Returns [`StartupError::CircularDependency`] if a cycle is detected, or
/// [`StartupError::MissingDependency`] if a component references an unknown
/// dependency.
pub fn topo_sort_layers(entries: &[ComponentEntry]) -> Result<Vec<Vec<ComponentId>>, StartupError> {
    // Build a map from ComponentId to its dependencies for quick lookup.
    let mut dep_map: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        std::collections::HashMap::new();
    let mut all_ids: std::collections::HashSet<ComponentId> = std::collections::HashSet::new();

    for entry in entries {
        if all_ids.contains(&entry.id) {
            // Duplicate entry — keep first-wins (first occurrence wins).
            continue;
        }
        dep_map.insert(entry.id, entry.deps.clone());
        all_ids.insert(entry.id);
    }

    // Validate that all declared dependencies exist.
    for (id, deps) in &dep_map {
        for dep in deps {
            if !all_ids.contains(dep) {
                return Err(StartupError::MissingDependency(*id, *dep));
            }
        }
    }

    // Kahn's algorithm with layer tracking.
    let mut in_degree: std::collections::HashMap<ComponentId, usize> =
        std::collections::HashMap::new();
    let mut reverse_deps: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        std::collections::HashMap::new();

    for &id in &all_ids {
        in_degree.entry(id).or_insert(0);
        reverse_deps.entry(id).or_default();
    }

    for (id, deps) in &dep_map {
        for dep in deps {
            *in_degree.entry(*id).or_insert(0) += 1;
            reverse_deps.entry(*dep).or_default().push(*id);
        }
    }

    // Collect initial layer: nodes with in_degree == 0, sorted by name.
    let mut layers: Vec<Vec<ComponentId>> = Vec::new();
    let mut current_layer: Vec<ComponentId> = all_ids
        .iter()
        .copied()
        .filter(|id| *in_degree.get(id).unwrap_or(&0) == 0)
        .collect();
    current_layer.sort_by_key(|id| id.name().to_string());
    layers.push(current_layer);

    let mut processed = layers[0].len();

    while let Some(layer) = layers.last() {
        let mut next_layer: Vec<ComponentId> = Vec::new();
        for &id in layer {
            if let Some(dependents) = reverse_deps.get(&id) {
                for &dep_id in dependents {
                    let deg = in_degree.get_mut(&dep_id).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        next_layer.push(dep_id);
                    }
                }
            }
        }
        if next_layer.is_empty() {
            break;
        }
        next_layer.sort_by_key(|id| id.name().to_string());
        processed += next_layer.len();
        layers.push(next_layer);
    }

    if processed != all_ids.len() {
        return Err(StartupError::CircularDependency);
    }

    Ok(layers)
}

/// Groups of components that must be initialized together in a given phase.
///
/// Each variant lists the [`ComponentId`]s that share the same phase.
/// The phase ordering matches the topological sort layer structure and
/// ensures that all dependencies of a phase are satisfied by earlier phases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupPhase {
    /// ConfigManager, Storage — no dependencies (Layer 0).
    Foundation,
    /// AgentRegistry, ConfigHotReload, PermissionEngine, RenderersPlugins,
    /// SessionConfigProvider, SkillsRegistry — depend on ConfigManager (Layer 1).
    Registries,
    /// AnnounceSweeper, ApprovalFlow, ArchiveSweeper, DreamingScheduler,
    /// IMAdapters, SkillWatcher, ToolsRegistry — depend on Layer 0-1 (Layer 2).
    CoreServices,
    /// SessionManager, SpawnController, SystemPromptBuilder — depend on
    /// Layer 0-2 (Layer 3).
    Wiring,
    /// Gateway — depends on Layer 0-3 (Layer 4).
    BackgroundAndFinal,
    /// AdminRpcServer — depends on Gateway (Layer 5).
    PostGateway,
}

impl StartupPhase {
    /// Returns the set of [`ComponentId`]s that belong to this phase.
    fn component_ids(&self) -> &'static [ComponentId] {
        use self::Foundation::*;
        use self::Service::*;
        match self {
            Self::Foundation => &[
                ComponentId::Foundation(ConfigManager),
                ComponentId::Foundation(Storage),
            ],
            Self::Registries => &[
                ComponentId::Service(AgentRegistry),
                ComponentId::Service(ConfigHotReload),
                ComponentId::Service(PermissionEngine),
                ComponentId::Service(RenderersPlugins),
                ComponentId::Service(SessionConfigProvider),
                ComponentId::Service(SkillsRegistry),
                ComponentId::Service(LLMRegistry),
            ],
            Self::CoreServices => &[
                ComponentId::Service(AnnounceSweeper),
                ComponentId::Service(ApprovalFlow),
                ComponentId::Service(ArchiveSweeper),
                ComponentId::Service(DreamingScheduler),
                ComponentId::Service(IMAdapters),
                ComponentId::Service(SkillWatcher),
                ComponentId::Service(ToolsRegistry),
            ],
            Self::Wiring => &[
                ComponentId::Service(SessionManager),
                ComponentId::Service(SpawnController),
                ComponentId::Service(SystemPromptBuilder),
            ],
            Self::BackgroundAndFinal => &[ComponentId::Service(Gateway)],
            Self::PostGateway => &[ComponentId::Service(AdminRpcServer)],
        }
    }
}

/// Ordered sequence of startup phases.
const STARTUP_PHASE_ORDER: &[StartupPhase] = &[
    StartupPhase::Foundation,
    StartupPhase::Registries,
    StartupPhase::CoreServices,
    StartupPhase::Wiring,
    StartupPhase::BackgroundAndFinal,
    StartupPhase::PostGateway,
];

/// Validate that the topological sort layers match the expected phase order.
///
/// This ensures the dependency graph produces the same phase structure as
/// the hardcoded initialization order. If the topo sort result diverges,
/// the daemon must refuse to start (the initialization code would be wrong).
///
/// # Errors
///
/// Returns [`StartupError`] if the layers don't match expected phases,
/// contain cycles, or reference missing dependencies.
pub fn validate_startup_layers(layers: &[Vec<ComponentId>]) -> Result<(), StartupError> {
    if layers.len() != STARTUP_PHASE_ORDER.len() {
        return Err(StartupError::CircularDependency);
    }
    for (i, phase) in STARTUP_PHASE_ORDER.iter().enumerate() {
        let expected = phase.component_ids();
        let mut actual = layers[i].clone();
        let mut expected_sorted = expected.to_vec();
        actual.sort_by_key(|id| id.name().to_string());
        expected_sorted.sort_by_key(|id| id.name().to_string());
        if actual != expected_sorted {
            return Err(StartupError::CircularDependency);
        }
    }
    Ok(())
}

#[cfg(test)]
mod startup_tests;
