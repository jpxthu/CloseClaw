//! Startup orchestration: component dependency declarations and data structures.
//!
//! Defines [`ComponentId`] to identify each daemon component and [`ComponentDeps`]
//! to declare startup dependencies. The topological sort engine (see
//! [`topo_sort_layers`]) consumes these declarations to derive the deterministic
//! initialization order.

/// Identifies a daemon component for startup orchestration.
///
/// Each variant corresponds to a component declared in the design doc dependency
/// table. The `name()` method provides a stable, human-readable label used for
/// alphabetical ordering within each layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentId {
    /// Loads and merges configuration files.
    ConfigManager,
    /// SQLite-backed session persistence.
    Storage,
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
}

impl ComponentId {
    /// Stable display name for this component.
    ///
    /// Used as the sort key for deterministic layer-internal ordering.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ConfigManager => "ConfigManager",
            Self::Storage => "Storage",
            Self::SessionConfigProvider => "SessionConfigProvider",
            Self::AgentRegistry => "AgentRegistry",
            Self::SkillsRegistry => "SkillsRegistry",
            Self::RenderersPlugins => "RenderersPlugins",
            Self::IMAdapters => "IMAdapters",
            Self::PermissionEngine => "PermissionEngine",
            Self::ToolsRegistry => "ToolsRegistry",
            Self::ArchiveSweeper => "ArchiveSweeper",
            Self::AnnounceSweeper => "AnnounceSweeper",
            Self::SkillWatcher => "SkillWatcher",
            Self::ConfigHotReload => "ConfigHotReload",
            Self::DreamingScheduler => "DreamingScheduler",
            Self::SessionManager => "SessionManager",
            Self::SystemPromptBuilder => "SystemPromptBuilder",
            Self::ApprovalFlow => "ApprovalFlow",
            Self::Gateway => "Gateway",
            Self::SpawnController => "SpawnController",
            Self::AdminRpcServer => "AdminRpcServer",
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
        use ComponentId::*;
        match self {
            ConfigManager => &[],
            Storage => &[],
            SessionConfigProvider => &[ConfigManager],
            AgentRegistry => &[ConfigManager],
            SkillsRegistry => &[ConfigManager],
            RenderersPlugins => &[ConfigManager],
            IMAdapters => &[RenderersPlugins, ConfigManager],
            PermissionEngine => &[AgentRegistry],
            ToolsRegistry => &[SkillsRegistry],
            ArchiveSweeper => &[Storage, SessionConfigProvider],
            AnnounceSweeper => &[Storage, SessionConfigProvider],
            SkillWatcher => &[SkillsRegistry],
            ConfigHotReload => &[ConfigManager],
            DreamingScheduler => &[Storage, SessionConfigProvider],
            SessionManager => &[Storage, AgentRegistry, SkillsRegistry, ToolsRegistry],
            SystemPromptBuilder => &[AgentRegistry, SkillsRegistry],
            ApprovalFlow => &[PermissionEngine, AgentRegistry],
            Gateway => &[SessionManager, IMAdapters, PermissionEngine, ApprovalFlow],
            SpawnController => &[AgentRegistry],
            AdminRpcServer => &[Gateway],
        }
    }
}

/// Returns [`ComponentEntry`]s for all 19 daemon components.
///
/// Each entry bundles the component identity, its human-readable name,
/// and the dependencies declared via [`ComponentDeps`].
pub fn all_component_entries() -> Vec<ComponentEntry> {
    use ComponentId::*;
    [
        ConfigManager,
        Storage,
        SessionConfigProvider,
        AgentRegistry,
        SkillsRegistry,
        RenderersPlugins,
        IMAdapters,
        PermissionEngine,
        ToolsRegistry,
        ArchiveSweeper,
        AnnounceSweeper,
        SkillWatcher,
        ConfigHotReload,
        DreamingScheduler,
        SessionManager,
        SystemPromptBuilder,
        ApprovalFlow,
        Gateway,
        SpawnController,
        AdminRpcServer,
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
    /// ConfigManager, Storage — no dependencies.
    Foundation,
    /// AgentRegistry, SkillsRegistry, ToolsRegistry — depend on ConfigManager.
    Registries,
    /// SessionManager, Gateway setup — depend on registries.
    CoreServices,
    /// SlashDispatcher, IM plugins, shutdown coordinator.
    Wiring,
    /// ArchiveSweeper, DreamingScheduler, registry population, approval flow.
    BackgroundAndFinal,
    /// SpawnController, AdminRpcServer — depend on Gateway.
    PostGateway,
}

impl StartupPhase {
    /// Returns the set of [`ComponentId`]s that belong to this phase.
    fn component_ids(&self) -> &'static [ComponentId] {
        use ComponentId::*;
        match self {
            Self::Foundation => &[ConfigManager, Storage],
            Self::Registries => &[
                AgentRegistry,
                ConfigHotReload,
                RenderersPlugins,
                SessionConfigProvider,
                SkillsRegistry,
            ],
            Self::CoreServices => &[
                ArchiveSweeper,
                AnnounceSweeper,
                DreamingScheduler,
                IMAdapters,
                PermissionEngine,
                SkillWatcher,
                SpawnController,
                SystemPromptBuilder,
                ToolsRegistry,
            ],
            Self::Wiring => &[ApprovalFlow, SessionManager],
            Self::BackgroundAndFinal => &[Gateway],
            Self::PostGateway => &[AdminRpcServer],
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
