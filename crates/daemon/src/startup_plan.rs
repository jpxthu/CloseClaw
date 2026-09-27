//! Topological startup orchestration.
//!
//! Extracted from `mod.rs` to keep source files within the CONTRIBUTING.md
//! limits (`mod.rs` only holds `pub use` / `pub mod` re-exports).

use crate::startup::{all_component_entries, topo_sort_layers, ComponentId, StartupError};
use tracing::info;

/// Resolved startup plan: topo-sort layers plus validated phase components.
/// Each outer element is a layer/phase; each inner element is a [`ComponentId`].
pub(crate) type StartupPlan = (Vec<Vec<ComponentId>>, Vec<Vec<ComponentId>>);

impl super::Daemon {
    /// Resolve the deterministic startup order from the component dependency
    /// graph. Returns topo-sorted layers; errors on circular dependency.
    pub(super) fn resolve_startup_order() -> Result<StartupPlan, StartupError> {
        let entries = all_component_entries();
        let layers = topo_sort_layers(&entries)?;
        let phase_components = Self::validate_phase_components(&layers)?;
        Ok((layers, phase_components))
    }
    /// Map each [`StartupPhase`] to its resolved [`ComponentId`] set,
    /// validated against the topo-sort result.
    pub(crate) fn validate_phase_components(
        layers: &[Vec<ComponentId>],
    ) -> Result<Vec<Vec<ComponentId>>, StartupError> {
        use crate::startup::{Foundation, Service};
        let c = |f: Foundation| ComponentId::Foundation(f);
        let s = |sv: Service| ComponentId::Service(sv);
        let expected: Vec<Vec<ComponentId>> = vec![
            vec![c(Foundation::ConfigManager), c(Foundation::Storage)],
            vec![
                s(Service::AgentRegistry),
                s(Service::ConfigHotReload),
                s(Service::PermissionEngine),
                s(Service::PlanArchiveSweeper),
                s(Service::RenderersPlugins),
                s(Service::SessionConfigProvider),
                s(Service::SkillsRegistry),
                s(Service::LLMRegistry),
            ],
            vec![
                s(Service::AnnounceSweeper),
                s(Service::ApprovalFlow),
                s(Service::ArchiveSweeper),
                s(Service::DreamingScheduler),
                s(Service::IMAdapters),
                s(Service::ToolsRegistry),
            ],
            vec![
                s(Service::SessionManager),
                s(Service::SpawnController),
                s(Service::SystemPromptBuilder),
            ],
            vec![s(Service::Gateway)],
            vec![s(Service::AdminRpcServer)],
        ];
        for (i, exp) in expected.iter().enumerate() {
            let mut actual = layers.get(i).cloned().unwrap_or_default();
            let mut exp_sorted = exp.clone();
            actual.sort_by_key(|id| id.name().to_string());
            exp_sorted.sort_by_key(|id| id.name().to_string());
            if actual != exp_sorted {
                return Err(StartupError::CircularDependency);
            }
        }
        Ok(expected)
    }
    /// Log the resolved startup order at `info` level for operational visibility.
    pub(super) fn log_startup_order(layers: &[Vec<ComponentId>]) {
        for (i, layer) in layers.iter().enumerate() {
            let names: Vec<&str> = layer.iter().map(|id| id.name()).collect();
            info!(layer = i + 1, components = ?names, "startup layer resolved");
        }
    }
}
