//! Expected startup phases and validation of topological layers against them.

use super::component::{ComponentId, Foundation, Service};
use super::graph::StartupError;

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
    /// IMAdapters, ToolsRegistry — depend on Layer 0-1 (Layer 2).
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
                ComponentId::Service(PlanArchiveSweeper),
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
