use super::*;
use Foundation::*;
use Service::*;

/// Helper: wrap a `Foundation` variant into `ComponentId`.
fn fid(f: Foundation) -> ComponentId {
    ComponentId::Foundation(f)
}

/// Helper: wrap a `Service` variant into `ComponentId`.
fn sid(s: Service) -> ComponentId {
    ComponentId::Service(s)
}

#[test]
fn test_component_id_name() {
    assert_eq!(fid(ConfigManager).name(), "ConfigManager");
    assert_eq!(sid(Gateway).name(), "Gateway");
    assert_eq!(sid(DreamingScheduler).name(), "DreamingScheduler");
    assert_eq!(sid(LLMRegistry).name(), "LLMRegistry");
}

#[test]
fn test_all_component_entries_count() {
    let entries = all_component_entries();
    assert_eq!(
        entries.len(),
        21,
        "expected 21 components (20 original + LLMRegistry)"
    );
}

#[test]
fn test_all_component_entries_deps_match_design_doc() {
    let entries = all_component_entries();

    // Build a lookup map for quick assertion
    let dep_map: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        entries.iter().map(|e| (e.id, e.deps.clone())).collect();

    // Layer 1: no deps
    assert_eq!(dep_map[&fid(ConfigManager)], vec![]);
    assert_eq!(dep_map[&fid(Storage)], vec![]);

    // Layer 2: depend on ConfigManager only
    assert_eq!(
        dep_map[&sid(SessionConfigProvider)],
        vec![fid(ConfigManager)]
    );
    assert_eq!(dep_map[&sid(AgentRegistry)], vec![fid(ConfigManager)]);
    assert_eq!(dep_map[&sid(SkillsRegistry)], vec![fid(ConfigManager)]);
    assert_eq!(dep_map[&sid(RenderersPlugins)], vec![fid(ConfigManager)]);
    assert_eq!(dep_map[&sid(PermissionEngine)], vec![fid(ConfigManager)]);
    assert_eq!(dep_map[&sid(LLMRegistry)], vec![fid(ConfigManager)]);

    // Layer 3
    assert_eq!(
        dep_map[&sid(IMAdapters)],
        vec![sid(RenderersPlugins), fid(ConfigManager)]
    );
    assert_eq!(dep_map[&sid(ToolsRegistry)], vec![sid(SkillsRegistry)]);
    assert_eq!(
        dep_map[&sid(ArchiveSweeper)],
        vec![fid(Storage), sid(SessionConfigProvider)]
    );
    assert_eq!(
        dep_map[&sid(AnnounceSweeper)],
        vec![fid(Storage), sid(SessionConfigProvider)]
    );
    assert_eq!(dep_map[&sid(SkillWatcher)], vec![sid(SkillsRegistry)]);
    assert_eq!(dep_map[&sid(ConfigHotReload)], vec![fid(ConfigManager)]);
    assert_eq!(
        dep_map[&sid(DreamingScheduler)],
        vec![fid(Storage), sid(SessionConfigProvider)]
    );
    assert_eq!(
        dep_map[&sid(SpawnController)],
        vec![sid(AgentRegistry), sid(ToolsRegistry)]
    );

    // Layer 3 (SystemPromptBuilder)
    assert_eq!(
        dep_map[&sid(SystemPromptBuilder)],
        vec![sid(AgentRegistry), sid(SkillsRegistry), sid(ToolsRegistry)]
    );
    // Layer 4
    assert_eq!(
        dep_map[&sid(SessionManager)],
        vec![
            sid(LLMRegistry),
            fid(Storage),
            sid(AgentRegistry),
            sid(SkillsRegistry),
            sid(ToolsRegistry),
            sid(SessionConfigProvider)
        ]
    );
    assert_eq!(
        dep_map[&sid(ApprovalFlow)],
        vec![sid(PermissionEngine), sid(AgentRegistry)]
    );

    // Layer 5
    assert_eq!(
        dep_map[&sid(Gateway)],
        vec![
            sid(SessionManager),
            sid(IMAdapters),
            sid(PermissionEngine),
            sid(ApprovalFlow),
            sid(RenderersPlugins)
        ]
    );
    assert_eq!(dep_map[&sid(AdminRpcServer)], vec![sid(Gateway)]);
}

#[test]
fn test_topo_sort_six_layers_match_design_doc() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    assert_eq!(layers.len(), 6, "expected exactly 6 layers");

    // Layer 1: ConfigManager, Storage (alphabetical)
    assert_eq!(
        layers[0],
        vec![fid(ConfigManager), fid(Storage)],
        "Layer 1 mismatch"
    );

    // Layer 2: components that depend only on ConfigManager (Layer 1)
    assert_eq!(
        layers[1],
        vec![
            sid(AgentRegistry),
            sid(ConfigHotReload),
            sid(LLMRegistry),
            sid(PermissionEngine),
            sid(RenderersPlugins),
            sid(SessionConfigProvider),
            sid(SkillsRegistry),
        ],
        "Layer 2 mismatch"
    );

    // Layer 3: components that depend on Layer 0-1
    assert_eq!(
        layers[2],
        vec![
            sid(AnnounceSweeper),
            sid(ApprovalFlow),
            sid(ArchiveSweeper),
            sid(DreamingScheduler),
            sid(IMAdapters),
            sid(SkillWatcher),
            sid(ToolsRegistry),
        ],
        "Layer 3 mismatch"
    );

    // Layer 4: SessionManager, SpawnController, SystemPromptBuilder
    assert_eq!(
        layers[3],
        vec![
            sid(SessionManager),
            sid(SpawnController),
            sid(SystemPromptBuilder)
        ],
        "Layer 4 mismatch"
    );

    // Layer 5: Gateway
    assert_eq!(layers[4], vec![sid(Gateway)], "Layer 5 mismatch");

    // Layer 6: AdminRpcServer (depends on Gateway)
    assert_eq!(layers[5], vec![sid(AdminRpcServer)], "Layer 6 mismatch");
}

// --------------------------------------------------------------------------
// Helper: build a ComponentEntry with a given id, name, and deps.
// --------------------------------------------------------------------------

fn entry(id: ComponentId, name: &'static str, deps: Vec<ComponentId>) -> ComponentEntry {
    ComponentEntry { id, name, deps }
}

// --------------------------------------------------------------------------
// Circular dependency detection
// --------------------------------------------------------------------------

#[test]
fn test_circular_dependency_a_b_c_a() {
    // A → B → C → A  (all three present → cycle)
    let e_a = entry(fid(ConfigManager), "A", vec![fid(Storage)]);
    let e_b = entry(fid(Storage), "B", vec![sid(Gateway)]);
    let e_c = entry(sid(Gateway), "C", vec![fid(ConfigManager)]);

    let err = topo_sort_layers(&[e_a, e_b, e_c]).unwrap_err();
    assert!(
        matches!(err, StartupError::CircularDependency),
        "expected CircularDependency, got: {err:?}"
    );
}

#[test]
fn test_circular_dependency_self_loop() {
    // A → A  (self-loop)
    let e_a = entry(fid(ConfigManager), "A", vec![fid(ConfigManager)]);

    let err = topo_sort_layers(&[e_a]).unwrap_err();
    assert!(
        matches!(err, StartupError::CircularDependency),
        "expected CircularDependency for self-loop, got: {err:?}"
    );
}

// --------------------------------------------------------------------------
// Missing dependency detection
// --------------------------------------------------------------------------

#[test]
fn test_missing_dependency_single() {
    // A depends on X (X not in entries)
    let e_a = entry(
        sid(AgentRegistry),
        "A",
        vec![sid(DreamingScheduler)], // DreamingScheduler not in this set
    );

    let err = topo_sort_layers(&[e_a]).unwrap_err();
    assert!(
        matches!(
            err,
            StartupError::MissingDependency(
                ComponentId::Service(Service::AgentRegistry),
                ComponentId::Service(Service::DreamingScheduler)
            )
        ),
        "expected MissingDependency(AgentRegistry, DreamingScheduler), got: {err:?}"
    );
}

#[test]
fn test_missing_dependency_multiple_unknown() {
    // A depends on B and X; B exists, X does not
    let e_a = entry(
        sid(AgentRegistry),
        "A",
        vec![fid(ConfigManager), sid(DreamingScheduler)],
    );
    let e_b = entry(fid(ConfigManager), "B", vec![]);

    let err = topo_sort_layers(&[e_a, e_b]).unwrap_err();
    assert!(
        matches!(
            err,
            StartupError::MissingDependency(
                ComponentId::Service(Service::AgentRegistry),
                ComponentId::Service(Service::DreamingScheduler)
            )
        ),
        "expected MissingDependency, got: {err:?}"
    );
}

// --------------------------------------------------------------------------
// Single node, no dependencies
// --------------------------------------------------------------------------

#[test]
fn test_single_node_no_deps() {
    let e = entry(fid(ConfigManager), "Solo", vec![]);
    let layers = topo_sort_layers(&[e]).expect("should succeed");

    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0], vec![fid(ConfigManager)]);
}

// --------------------------------------------------------------------------
// Empty input
// --------------------------------------------------------------------------

#[test]
fn test_empty_input() {
    let layers = topo_sort_layers(&[]).expect("empty input should succeed");
    assert!(
        layers.len() <= 1,
        "empty input should produce at most 1 layer"
    );
    if let Some(first) = layers.first() {
        assert!(first.is_empty(), "empty input layer should be empty");
    }
}

// --------------------------------------------------------------------------
// Diamond dependency
// --------------------------------------------------------------------------

#[test]
fn test_diamond_dependency() {
    // Diamond: A at top, B and C in middle, D at bottom.
    // A -> nothing, B -> A, C -> A, D -> B and C
    let e_a = entry(fid(ConfigManager), "A", vec![]);
    let e_b = entry(fid(Storage), "B", vec![fid(ConfigManager)]);
    let e_c = entry(sid(Gateway), "C", vec![fid(ConfigManager)]);
    let e_d = entry(sid(AgentRegistry), "D", vec![fid(Storage), sid(Gateway)]);

    let layers = topo_sort_layers(&[e_a, e_b, e_c, e_d]).expect("diamond should succeed");

    // Expected layers:
    //   L0: [A]                       (no deps)
    //   L1: [B, C]                    (depend only on A, sorted by name)
    //   L2: [D]                       (depends on B and C)
    assert_eq!(layers.len(), 3, "diamond should produce 3 layers");
    assert_eq!(layers[0], vec![fid(ConfigManager)], "L0 should be [A]");
    // B = Storage, C = Gateway → alphabetical by name() = Gateway, Storage
    assert_eq!(
        layers[1],
        vec![sid(Gateway), fid(Storage)],
        "L1 should be [C, B] sorted"
    );
    assert_eq!(layers[2], vec![sid(AgentRegistry)], "L2 should be [D]");
}

#[test]
fn test_diamond_dependency_alphabetical_in_layer() {
    // Verify that within a layer, items are sorted alphabetically by name().
    // Provide entries in reverse order to ensure sort, not insertion order.
    let e_d = entry(sid(AgentRegistry), "D", vec![fid(Storage), sid(Gateway)]);
    let e_c = entry(sid(Gateway), "C", vec![fid(ConfigManager)]);
    let e_b = entry(fid(Storage), "B", vec![fid(ConfigManager)]);
    let e_a = entry(fid(ConfigManager), "A", vec![]);

    let layers = topo_sort_layers(&[e_d, e_c, e_b, e_a]).expect("diamond should succeed");

    // L1 must be sorted by name: C=Gateway < B=Storage
    assert_eq!(layers[1], vec![sid(Gateway), fid(Storage)]);
}

// --------------------------------------------------------------------------
// SpawnController and AdminRpcServer dependency validation
// --------------------------------------------------------------------------

#[test]
fn test_spawn_controller_depends_on_agent_registry() {
    let entries = all_component_entries();
    let dep_map: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        entries.iter().map(|e| (e.id, e.deps.clone())).collect();

    assert_eq!(
        dep_map[&sid(SpawnController)],
        vec![sid(AgentRegistry), sid(ToolsRegistry)],
        "SpawnController must depend on AgentRegistry and ToolsRegistry per design doc Layer 4"
    );
}

#[test]
fn test_admin_rpc_server_depends_on_gateway() {
    let entries = all_component_entries();
    let dep_map: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        entries.iter().map(|e| (e.id, e.deps.clone())).collect();

    assert_eq!(
        dep_map[&sid(AdminRpcServer)],
        vec![sid(Gateway)],
        "AdminRpcServer must depend on Gateway per design doc Layer 5/6"
    );
}

#[test]
fn test_spawn_controller_in_core_services_layer() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    // SpawnController is in Layer 4 (Wiring phase)
    // Layer index 3 = fourth layer
    assert!(
        layers[3].contains(&sid(SpawnController)),
        "SpawnController must be in Layer 4 (Wiring), got layers: {:?}",
        layers
            .iter()
            .enumerate()
            .map(|(i, l)| (i, l.iter().map(|c| c.name()).collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_admin_rpc_server_in_post_gateway_layer() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    // AdminRpcServer is in Layer 6 (PostGateway phase)
    // Layer index 5 = sixth layer
    assert!(
        layers[5].contains(&sid(AdminRpcServer)),
        "AdminRpcServer must be in Layer 6 (PostGateway), got layers: {:?}",
        layers
            .iter()
            .enumerate()
            .map(|(i, l)| (i, l.iter().map(|c| c.name()).collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_validate_layers_catches_wrong_spawn_controller_layer() {
    // Manually build layers with SpawnController misplaced into Layer 1
    let wrong_layers: Vec<Vec<ComponentId>> = vec![
        vec![fid(ConfigManager), fid(Storage), sid(SpawnController)], // Wrong: SpawnController here
        vec![
            sid(AgentRegistry),
            sid(ConfigHotReload),
            sid(RenderersPlugins),
            sid(SessionConfigProvider),
            sid(SkillsRegistry),
            sid(LLMRegistry),
        ],
        vec![
            sid(AnnounceSweeper),
            sid(ArchiveSweeper),
            sid(DreamingScheduler),
            sid(IMAdapters),
            sid(PermissionEngine),
            sid(SkillWatcher),
            sid(ToolsRegistry),
        ],
        vec![
            sid(ApprovalFlow),
            sid(SessionManager),
            sid(SystemPromptBuilder),
        ],
        vec![sid(Gateway)],
        vec![sid(AdminRpcServer)],
    ];
    let err = validate_startup_layers(&wrong_layers).unwrap_err();
    assert!(
        matches!(err, StartupError::CircularDependency),
        "validation should reject wrong SpawnController layer placement"
    );
}

#[test]
fn test_validate_layers_catches_wrong_admin_rpc_server_layer() {
    // Manually build layers with AdminRpcServer misplaced into Layer 4
    let wrong_layers: Vec<Vec<ComponentId>> = vec![
        vec![fid(ConfigManager), fid(Storage)],
        vec![
            sid(AgentRegistry),
            sid(ConfigHotReload),
            sid(LLMRegistry),
            sid(RenderersPlugins),
            sid(SessionConfigProvider),
            sid(SkillsRegistry),
        ],
        vec![
            sid(AnnounceSweeper),
            sid(ArchiveSweeper),
            sid(DreamingScheduler),
            sid(IMAdapters),
            sid(PermissionEngine),
            sid(SkillWatcher),
            sid(SpawnController),
            sid(ToolsRegistry),
        ],
        vec![
            sid(ApprovalFlow),
            sid(SessionManager),
            sid(SystemPromptBuilder),
            sid(AdminRpcServer),
        ], // Wrong: AdminRpcServer here
        vec![sid(Gateway)],
        vec![],
    ];
    let err = validate_startup_layers(&wrong_layers).unwrap_err();
    assert!(
        matches!(err, StartupError::CircularDependency),
        "validation should reject wrong AdminRpcServer layer placement"
    );
}

// --------------------------------------------------------------------------
// Layer-internal alphabetical ordering (full sort order)
// --------------------------------------------------------------------------

#[test]
fn test_validate_startup_layers_succeeds() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    validate_startup_layers(&layers).expect("validation should succeed");
}

#[test]
fn test_validate_startup_layers_wrong_count() {
    // Only 2 layers instead of 5 — should fail.
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    let truncated = &layers[..2];
    let err = validate_startup_layers(truncated).unwrap_err();
    assert!(matches!(err, StartupError::CircularDependency));
}

#[test]
fn test_validate_startup_layers_wrong_order() {
    // Swap layer 1 and layer 2 — should fail.
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    let mut swapped = layers.clone();
    swapped.swap(0, 1);
    let err = validate_startup_layers(&swapped).unwrap_err();
    assert!(matches!(err, StartupError::CircularDependency));
}

// --------------------------------------------------------------------------
// Layer-internal alphabetical ordering (full sort order)
// --------------------------------------------------------------------------

#[test]
fn test_layer_internal_alphabetical_order() {
    // Three independent nodes → should all be in L0, sorted by name.
    let e_a = entry(sid(AgentRegistry), "Zebra", vec![]);
    let e_b = entry(fid(ConfigManager), "Apple", vec![]);
    let e_c = entry(fid(Storage), "Mango", vec![]);

    let layers = topo_sort_layers(&[e_a, e_b, e_c]).expect("should succeed");
    // Three independent nodes → one layer, sorted by id.name()
    // AgentRegistry < ConfigManager < Storage
    assert_eq!(layers.len(), 1);
    assert_eq!(
        layers[0],
        vec![sid(AgentRegistry), fid(ConfigManager), fid(Storage)]
    );
}

// --------------------------------------------------------------------------
// Linear chain: A → B → C → D
// --------------------------------------------------------------------------

#[test]
fn test_linear_chain() {
    // Each depends on the previous; should produce 4 layers.
    let e_d = entry(sid(AgentRegistry), "D", vec![fid(Storage)]);
    let e_c = entry(fid(Storage), "C", vec![sid(Gateway)]);
    let e_b = entry(sid(Gateway), "B", vec![fid(ConfigManager)]);
    let e_a = entry(fid(ConfigManager), "A", vec![]);

    let layers = topo_sort_layers(&[e_d, e_c, e_b, e_a]).expect("linear chain should succeed");

    assert_eq!(layers.len(), 4);
    assert_eq!(layers[0], vec![fid(ConfigManager)]);
    assert_eq!(layers[1], vec![sid(Gateway)]);
    assert_eq!(layers[2], vec![fid(Storage)]);
    assert_eq!(layers[3], vec![sid(AgentRegistry)]);
}

// --------------------------------------------------------------------------
// All nodes in parallel (no deps between any pair)
// --------------------------------------------------------------------------

#[test]
fn test_all_parallel() {
    let entries = vec![
        entry(fid(ConfigManager), "C", vec![]),
        entry(sid(AgentRegistry), "A", vec![]),
        entry(fid(Storage), "B", vec![]),
    ];
    let layers = topo_sort_layers(&entries).expect("all parallel should succeed");

    assert_eq!(layers.len(), 1);
    // A < B < C by id.name(): AgentRegistry < ConfigManager < Storage
    assert_eq!(
        layers[0],
        vec![sid(AgentRegistry), fid(ConfigManager), fid(Storage)]
    );
}

// ======================================================================
// Step 1.3 — SystemPromptBuilder layer and dependency tests
// ======================================================================

/// SystemPromptBuilder must reside in Layer 4 (Wiring phase).
#[test]
fn test_system_prompt_builder_in_core_services_layer() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    // Layer index 3 = fourth layer = Wiring
    assert!(
        layers[3].contains(&sid(SystemPromptBuilder)),
        "SystemPromptBuilder must be in Layer 4 (Wiring), got layers: {:?}",
        layers
            .iter()
            .enumerate()
            .map(|(i, l)| (i, l.iter().map(|c| c.name()).collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    );
}

/// SystemPromptBuilder dependencies must be exactly [AgentRegistry, SkillsRegistry, ToolsRegistry].
#[test]
fn test_system_prompt_builder_deps_only_agent_and_skills() {
    let entries = all_component_entries();
    let dep_map: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        entries.iter().map(|e| (e.id, e.deps.clone())).collect();

    let deps = &dep_map[&sid(SystemPromptBuilder)];
    assert_eq!(
        deps,
        &vec![sid(AgentRegistry), sid(SkillsRegistry), sid(ToolsRegistry)],
        "SystemPromptBuilder must depend on [AgentRegistry, SkillsRegistry, ToolsRegistry]"
    );
}

/// SystemPromptBuilder MUST depend on ToolsRegistry (design doc Layer 4).
#[test]
fn test_system_prompt_builder_no_tools_registry_dep() {
    let entries = all_component_entries();
    let dep_map: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        entries.iter().map(|e| (e.id, e.deps.clone())).collect();

    let deps = &dep_map[&sid(SystemPromptBuilder)];
    assert!(
        deps.contains(&sid(ToolsRegistry)),
        "SystemPromptBuilder must depend on ToolsRegistry (design doc Layer 4)"
    );
}

/// validate_phase_components passes when SystemPromptBuilder is in the
/// correct CoreServices phase (Layer 3).
#[test]
fn test_validate_phase_components_with_system_prompt_builder() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    let result = crate::Daemon::validate_phase_components(&layers);
    assert!(
        result.is_ok(),
        "validate_phase_components should succeed with SystemPromptBuilder in CoreServices: {:?}",
        result.err()
    );

    let phases = result.unwrap();
    // Phase 4 (index 3) = Wiring must contain SystemPromptBuilder
    assert!(
        phases[3].contains(&sid(SystemPromptBuilder)),
        "Phase 4 (Wiring) must contain SystemPromptBuilder"
    );
    // Phase 3 (index 2) = CoreServices must NOT contain SystemPromptBuilder
    assert!(
        !phases[2].contains(&sid(SystemPromptBuilder)),
        "Phase 3 (CoreServices) must NOT contain SystemPromptBuilder"
    );
}

// ======================================================================
// Step 1.4 — LLM Registry dependency-driven startup unit tests
// ======================================================================

// --- Normal path: all_component_entries + deps + topo sort ---

/// LLMRegistry must appear in `all_component_entries()`.
#[test]
fn test_llm_registry_present_in_all_component_entries() {
    let entries = all_component_entries();
    let ids: Vec<ComponentId> = entries.iter().map(|e| e.id).collect();
    assert!(
        ids.contains(&sid(LLMRegistry)),
        "all_component_entries() must contain LLMRegistry"
    );
}

/// LLMRegistry must be in layer 2 (Registries phase) of the topo sort.
#[test]
fn test_llm_registry_in_layer_two_of_topo_sort() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    // Layer index 1 = second layer = Registries phase
    assert!(
        layers[1].contains(&sid(LLMRegistry)),
        "LLMRegistry must be in layer 2 (Registries phase), got layers: {:?}",
        layers
            .iter()
            .enumerate()
            .map(|(i, l)| (i, l.iter().map(|c| c.name()).collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    );
}

/// LLMRegistry is NOT in layer 1 (Foundation phase).
#[test]
fn test_llm_registry_not_in_layer_one() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    assert!(
        !layers[0].contains(&sid(LLMRegistry)),
        "LLMRegistry must not be in layer 1 (Foundation phase)"
    );
}

/// SessionManager deps include LLMRegistry (design doc Layer 4 dependency).
#[test]
fn test_session_manager_deps_include_llm_registry() {
    let entries = all_component_entries();
    let dep_map: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        entries.iter().map(|e| (e.id, e.deps.clone())).collect();
    assert!(
        dep_map[&sid(SessionManager)].contains(&sid(LLMRegistry)),
        "SessionManager deps must include LLMRegistry, got: {:?}",
        dep_map[&sid(SessionManager)]
    );
}

/// validate_phase_components places LLMRegistry in Registries phase (index 1).
#[test]
fn test_validate_phase_components_places_llm_registry_in_registries() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    let phases = crate::Daemon::validate_phase_components(&layers)
        .expect("validate_phase_components should succeed");
    // Registries phase (index 1) must contain LLMRegistry
    assert!(
        phases[1].contains(&sid(LLMRegistry)),
        "Registries phase (index 1) must contain LLMRegistry"
    );
    // Foundation phase must NOT contain LLMRegistry
    assert!(
        !phases[0].contains(&sid(LLMRegistry)),
        "Foundation phase (index 0) must NOT contain LLMRegistry"
    );
    // Wiring phase must NOT contain LLMRegistry
    assert!(
        !phases[3].contains(&sid(LLMRegistry)),
        "Wiring phase (index 3) must NOT contain LLMRegistry"
    );
}

// --- Boundary values: enum variant count ---

/// Foundation enum must have exactly 2 variants (within the 20-variant limit).
#[test]
fn test_foundation_enum_variant_count() {
    let entries = all_component_entries();
    let foundation_count = entries
        .iter()
        .filter(|e| matches!(e.id, ComponentId::Foundation(_)))
        .count();
    assert_eq!(
        foundation_count, 2,
        "Foundation must have exactly 2 variants (ConfigManager, Storage)"
    );
}

/// Service enum must have exactly 19 variants (total components = 21 ≤ 20 per enum).
#[test]
fn test_service_enum_variant_count() {
    let entries = all_component_entries();
    let service_count = entries
        .iter()
        .filter(|e| matches!(e.id, ComponentId::Service(_)))
        .count();
    assert_eq!(
        service_count, 19,
        "Service must have exactly 19 variants (total 21 components)"
    );
}

// --- Boundary values: validate_phase_components layer 2 consistency ---

/// validate_phase_components layer 2 (Registries) must include LLMRegistry
/// and match the actual topo sort layer 2.
#[test]
fn test_validate_phase_components_layer_two_matches_topo_sort() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    let phases = crate::Daemon::validate_phase_components(&layers)
        .expect("validate_phase_components should succeed");
    // Layer 2 components from topo sort
    let mut topo_layer2 = layers[1].clone();
    topo_layer2.sort_by_key(|id| id.name().to_string());
    // Phase components for Registries (index 1)
    let mut phase_registries = phases[1].clone();
    phase_registries.sort_by_key(|id| id.name().to_string());
    assert_eq!(
        topo_layer2, phase_registries,
        "Topo sort layer 2 must match Registries phase components"
    );
}

/// LLMRegistry must be in the Registries phase expected set.
#[test]
fn test_validate_phase_components_registries_expected_includes_llm_registry() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    let phases = crate::Daemon::validate_phase_components(&layers)
        .expect("validate_phase_components should succeed");
    let registries_phase = &phases[1];
    assert!(
        registries_phase.contains(&sid(LLMRegistry)),
        "Registries phase expected set must include LLMRegistry"
    );
}

// --- State transition: init_llm_registry produces a usable registry ---

/// init_llm_registry with empty env_overrides and no credentials files
/// must return an empty registry (no providers registered).
#[tokio::test]
async fn test_init_llm_registry_empty_env_returns_empty_registry() {
    let dir = tempfile::tempdir().unwrap();
    let registry =
        crate::Daemon::init_llm_registry(dir.path(), &std::collections::HashMap::new()).await;
    let providers = registry.list().await;
    assert!(
        providers.is_empty(),
        "init_llm_registry with no credentials must return empty registry"
    );
}

/// init_llm_registry with a specific env override must register that provider.
#[tokio::test]
async fn test_init_llm_registry_with_env_override_registers_provider() {
    let dir = tempfile::tempdir().unwrap();
    let overrides = std::collections::HashMap::from([("OPENAI_API_KEY", "sk-test")]);
    let registry = crate::Daemon::init_llm_registry(dir.path(), &overrides).await;
    let providers = registry.list().await;
    assert!(
        providers.contains(&"openai".to_string()),
        "init_llm_registry must register openai provider from env override"
    );
}

/// init_llm_registry must NOT register providers for empty API keys.
#[tokio::test]
async fn test_init_llm_registry_empty_key_not_registered() {
    let dir = tempfile::tempdir().unwrap();
    let overrides =
        std::collections::HashMap::from([("OPENAI_API_KEY", ""), ("ANTHROPIC_API_KEY", "")]);
    let registry = crate::Daemon::init_llm_registry(dir.path(), &overrides).await;
    let providers = registry.list().await;
    assert!(
        providers.is_empty(),
        "init_llm_registry must not register providers for empty keys"
    );
}

// --- Error path: circular dependency detection with LLMRegistry ---

/// A dependency graph where A → LLMRegistry → A must detect a cycle.
#[test]
fn test_circular_dependency_through_llm_registry() {
    // Build a custom graph: LLMRegistry depends on ConfigManager,
    // but ConfigManager depends on LLMRegistry (cycle)
    let e_llm = entry(sid(LLMRegistry), "LLMRegistry", vec![fid(ConfigManager)]);
    let e_cfg = entry(fid(ConfigManager), "ConfigManager", vec![sid(LLMRegistry)]);
    let err = topo_sort_layers(&[e_llm, e_cfg]).unwrap_err();
    assert!(
        matches!(err, StartupError::CircularDependency),
        "expected CircularDependency when LLMRegistry ↔ ConfigManager cycle, got: {err:?}"
    );
}

/// LLMRegistry in the full component graph must NOT introduce a cycle.
#[test]
fn test_full_graph_with_llm_registry_is_acyclic() {
    let entries = all_component_entries();
    let result = topo_sort_layers(&entries);
    assert!(
        result.is_ok(),
        "full component graph with LLMRegistry must be acyclic, got: {:?}",
        result.err()
    );
    let layers = result.unwrap();
    // All 21 components must be reachable (no cycle)
    let total: usize = layers.iter().map(|l| l.len()).sum();
    assert_eq!(
        total, 21,
        "all 21 components must be in topo sort result (no cycle)"
    );
}

/// LLMRegistry misplacement into Foundation phase must be rejected.
#[test]
fn test_validate_rejects_llm_registry_in_foundation_phase() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    // Swap LLMRegistry from layer 2 into layer 1 (Foundation)
    let mut wrong_layers = layers.clone();
    wrong_layers[1].retain(|id| *id != sid(LLMRegistry));
    wrong_layers[0].push(sid(LLMRegistry));
    wrong_layers[0].sort_by_key(|id| id.name().to_string());
    let err = crate::Daemon::validate_phase_components(&wrong_layers).unwrap_err();
    assert!(
        matches!(err, StartupError::CircularDependency),
        "validation must reject LLMRegistry in Foundation phase"
    );
}

/// LLMRegistry misplacement into Wiring phase must be rejected.
#[test]
fn test_validate_rejects_llm_registry_in_wiring_phase() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    // Swap LLMRegistry from layer 2 into layer 3 (Wiring)
    let mut wrong_layers = layers.clone();
    wrong_layers[1].retain(|id| *id != sid(LLMRegistry));
    wrong_layers[3].push(sid(LLMRegistry));
    wrong_layers[3].sort_by_key(|id| id.name().to_string());
    let err = crate::Daemon::validate_phase_components(&wrong_layers).unwrap_err();
    assert!(
        matches!(err, StartupError::CircularDependency),
        "validation must reject LLMRegistry in Wiring phase"
    );
}
