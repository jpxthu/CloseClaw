//! Topological startup orchestration.
//!
//! Extracted from `mod.rs` to keep source files within the CONTRIBUTING.md
//! limits (`mod.rs` only holds `pub use` / `pub mod` re-exports).

use crate::startup::{
    all_component_entries, layers_match_phase_components, phase_components, topo_sort_layers,
    ComponentId, StartupError,
};
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
        let expected = phase_components();
        if layers_match_phase_components(layers) {
            Ok(expected)
        } else {
            Err(StartupError::StartupLayersMismatch)
        }
    }
    /// Log the resolved startup order at `info` level for operational visibility.
    pub(super) fn log_startup_order(layers: &[Vec<ComponentId>]) {
        for (i, layer) in layers.iter().enumerate() {
            let names: Vec<&str> = layer.iter().map(|id| id.name()).collect();
            info!(layer = i + 1, components = ?names, "startup layer resolved");
        }
    }
}
