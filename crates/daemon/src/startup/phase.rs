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

/// Returns the expected [`ComponentId`] set of each startup phase.
///
/// This is the single source of truth for the phase-to-component mapping,
/// derived from [`STARTUP_PHASE_ORDER`] and [`StartupPhase::component_ids`].
pub(crate) fn phase_components() -> Vec<Vec<ComponentId>> {
    STARTUP_PHASE_ORDER
        .iter()
        .map(|phase| phase.component_ids().to_vec())
        .collect()
}

/// Compare resolved layers against [`phase_components`], order-insensitively
/// within each layer.
///
/// Returns `true` when the layer count matches and every layer contains
/// exactly the expected component set.
pub(crate) fn layers_match_phase_components(layers: &[Vec<ComponentId>]) -> bool {
    let expected = phase_components();
    if layers.len() != expected.len() {
        return false;
    }
    layers.iter().zip(expected.iter()).all(|(actual, exp)| {
        let mut actual_sorted = actual.clone();
        let mut exp_sorted = exp.clone();
        actual_sorted.sort_by_key(|id| id.name().to_string());
        exp_sorted.sort_by_key(|id| id.name().to_string());
        actual_sorted == exp_sorted
    })
}

/// Validate that the topological sort layers match the expected phase order.
///
/// This ensures the dependency graph produces the same phase structure as
/// the expected initialization order: the layer count and each layer's
/// component set (order-insensitive) must match [`phase_components`].
///
/// # Errors
///
/// Returns [`StartupError::StartupLayersMismatch`] when the layer count or
/// any layer's component set diverges from the expected phases. Circular
/// dependencies and missing dependencies are not detected here; they are
/// reported by [`super::graph::topo_sort_layers`].
pub fn validate_startup_layers(layers: &[Vec<ComponentId>]) -> Result<(), StartupError> {
    if layers_match_phase_components(layers) {
        Ok(())
    } else {
        Err(StartupError::StartupLayersMismatch)
    }
}
