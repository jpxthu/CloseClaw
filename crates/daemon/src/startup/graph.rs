//! Startup graph: full component inventory, topological layering, and errors.
//!
//! Consumes the declarations from `super::component` and produces the
//! deterministic initialization order used by daemon startup.

use super::component::{ComponentDeps, ComponentEntry, ComponentId, Foundation, Service};

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
        ComponentId::Service(Service::ConfigHotReload),
        ComponentId::Service(Service::DreamingScheduler),
        ComponentId::Service(Service::LLMRegistry),
        ComponentId::Service(Service::SessionManager),
        ComponentId::Service(Service::SystemPromptBuilder),
        ComponentId::Service(Service::ApprovalFlow),
        ComponentId::Service(Service::Gateway),
        ComponentId::Service(Service::SpawnController),
        ComponentId::Service(Service::PlanArchiveSweeper),
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
