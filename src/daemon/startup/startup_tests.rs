use super::*;

#[test]
fn test_component_id_name() {
    assert_eq!(ComponentId::ConfigManager.name(), "ConfigManager");
    assert_eq!(ComponentId::Gateway.name(), "Gateway");
    assert_eq!(ComponentId::DreamingScheduler.name(), "DreamingScheduler");
}

#[test]
fn test_all_component_entries_count() {
    let entries = all_component_entries();
    assert_eq!(
        entries.len(),
        19,
        "expected 19 components (17 original + SpawnController + AdminRpcServer)"
    );
}

#[test]
fn test_all_component_entries_deps_match_design_doc() {
    let entries = all_component_entries();

    // Build a lookup map for quick assertion
    let dep_map: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        entries.iter().map(|e| (e.id, e.deps.clone())).collect();

    use ComponentId::*;

    // Layer 1: no deps
    assert_eq!(dep_map[&ConfigManager], vec![]);
    assert_eq!(dep_map[&Storage], vec![]);

    // Layer 2: depend on ConfigManager only
    assert_eq!(dep_map[&SessionConfigProvider], vec![ConfigManager]);
    assert_eq!(dep_map[&AgentRegistry], vec![ConfigManager]);
    assert_eq!(dep_map[&SkillsRegistry], vec![ConfigManager]);
    assert_eq!(dep_map[&RenderersPlugins], vec![ConfigManager]);

    // Layer 3
    assert_eq!(dep_map[&IMAdapters], vec![RenderersPlugins, ConfigManager]);
    assert_eq!(dep_map[&PermissionEngine], vec![AgentRegistry]);
    assert_eq!(dep_map[&ToolsRegistry], vec![SkillsRegistry]);
    assert_eq!(
        dep_map[&ArchiveSweeper],
        vec![Storage, SessionConfigProvider]
    );
    assert_eq!(dep_map[&SkillWatcher], vec![SkillsRegistry]);
    assert_eq!(dep_map[&ConfigHotReload], vec![ConfigManager]);
    assert_eq!(
        dep_map[&DreamingScheduler],
        vec![Storage, SessionConfigProvider]
    );
    assert_eq!(dep_map[&SpawnController], vec![AgentRegistry]);

    // Layer 4
    assert_eq!(
        dep_map[&SessionManager],
        vec![Storage, AgentRegistry, SkillsRegistry, ToolsRegistry]
    );
    assert_eq!(
        dep_map[&SystemPromptBuilder],
        vec![AgentRegistry, SkillsRegistry]
    );
    assert_eq!(
        dep_map[&ApprovalFlow],
        vec![PermissionEngine, AgentRegistry]
    );

    // Layer 5
    assert_eq!(
        dep_map[&Gateway],
        vec![SessionManager, IMAdapters, PermissionEngine, ApprovalFlow]
    );
    assert_eq!(dep_map[&AdminRpcServer], vec![Gateway]);
}

#[test]
fn test_topo_sort_six_layers_match_design_doc() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    assert_eq!(layers.len(), 6, "expected exactly 6 layers");

    use ComponentId::*;

    // Layer 1: ConfigManager, Storage (alphabetical)
    assert_eq!(layers[0], vec![ConfigManager, Storage], "Layer 1 mismatch");

    // Layer 2: ConfigHotReload depends only on ConfigManager (Layer 1),
    // so Kahn's algorithm places it here, not Layer 3.
    assert_eq!(
        layers[1],
        vec![
            AgentRegistry,
            ConfigHotReload,
            RenderersPlugins,
            SessionConfigProvider,
            SkillsRegistry,
        ],
        "Layer 2 mismatch"
    );

    // Layer 3: ArchiveSweeper, DreamingScheduler, IMAdapters,
    //          PermissionEngine, SkillWatcher, SpawnController,
    //          SystemPromptBuilder, ToolsRegistry
    assert_eq!(
        layers[2],
        vec![
            ArchiveSweeper,
            DreamingScheduler,
            IMAdapters,
            PermissionEngine,
            SkillWatcher,
            SpawnController,
            SystemPromptBuilder,
            ToolsRegistry,
        ],
        "Layer 3 mismatch"
    );

    // Layer 4: ApprovalFlow, SessionManager
    assert_eq!(
        layers[3],
        vec![ApprovalFlow, SessionManager],
        "Layer 4 mismatch"
    );

    // Layer 5: Gateway
    assert_eq!(layers[4], vec![Gateway], "Layer 5 mismatch");

    // Layer 6: AdminRpcServer (depends on Gateway)
    assert_eq!(layers[5], vec![AdminRpcServer], "Layer 6 mismatch");
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
    let e_a = entry(ComponentId::ConfigManager, "A", vec![ComponentId::Storage]);
    let e_b = entry(ComponentId::Storage, "B", vec![ComponentId::Gateway]);
    let e_c = entry(ComponentId::Gateway, "C", vec![ComponentId::ConfigManager]);

    let err = topo_sort_layers(&[e_a, e_b, e_c]).unwrap_err();
    assert!(
        matches!(err, StartupError::CircularDependency),
        "expected CircularDependency, got: {err:?}"
    );
}

#[test]
fn test_circular_dependency_self_loop() {
    // A → A  (self-loop)
    let e_a = entry(
        ComponentId::ConfigManager,
        "A",
        vec![ComponentId::ConfigManager],
    );

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
    use ComponentId::*;
    let e_a = entry(
        AgentRegistry,
        "A",
        vec![DreamingScheduler], // DreamingScheduler not in this set
    );

    let err = topo_sort_layers(&[e_a]).unwrap_err();
    assert!(
        matches!(
            err,
            StartupError::MissingDependency(AgentRegistry, DreamingScheduler)
        ),
        "expected MissingDependency(AgentRegistry, DreamingScheduler), got: {err:?}"
    );
}

#[test]
fn test_missing_dependency_multiple_unknown() {
    // A depends on B and X; B exists, X does not
    use ComponentId::*;
    let e_a = entry(AgentRegistry, "A", vec![ConfigManager, DreamingScheduler]);
    let e_b = entry(ConfigManager, "B", vec![]);

    let err = topo_sort_layers(&[e_a, e_b]).unwrap_err();
    assert!(
        matches!(
            err,
            StartupError::MissingDependency(AgentRegistry, DreamingScheduler)
        ),
        "expected MissingDependency, got: {err:?}"
    );
}

// --------------------------------------------------------------------------
// Single node, no dependencies
// --------------------------------------------------------------------------

#[test]
fn test_single_node_no_deps() {
    use ComponentId::*;
    let e = entry(ConfigManager, "Solo", vec![]);
    let layers = topo_sort_layers(&[e]).expect("should succeed");

    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0], vec![ConfigManager]);
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
    use ComponentId::*;
    let e_a = entry(ConfigManager, "A", vec![]);
    let e_b = entry(Storage, "B", vec![ConfigManager]);
    let e_c = entry(Gateway, "C", vec![ConfigManager]);
    let e_d = entry(AgentRegistry, "D", vec![Storage, Gateway]);

    let layers = topo_sort_layers(&[e_a, e_b, e_c, e_d]).expect("diamond should succeed");

    // Expected layers:
    //   L0: [A]                       (no deps)
    //   L1: [B, C]                    (depend only on A, sorted by name)
    //   L2: [D]                       (depends on B and C)
    assert_eq!(layers.len(), 3, "diamond should produce 3 layers");
    assert_eq!(layers[0], vec![ConfigManager], "L0 should be [A]");
    // B = Storage, C = Gateway → alphabetical by name() = Gateway, Storage
    assert_eq!(
        layers[1],
        vec![Gateway, Storage],
        "L1 should be [C, B] sorted"
    );
    assert_eq!(layers[2], vec![AgentRegistry], "L2 should be [D]");
}

#[test]
fn test_diamond_dependency_alphabetical_in_layer() {
    // Verify that within a layer, items are sorted alphabetically by name().
    // Provide entries in reverse order to ensure sort, not insertion order.
    use ComponentId::*;
    let e_d = entry(AgentRegistry, "D", vec![Storage, Gateway]);
    let e_c = entry(Gateway, "C", vec![ConfigManager]);
    let e_b = entry(Storage, "B", vec![ConfigManager]);
    let e_a = entry(ConfigManager, "A", vec![]);

    let layers = topo_sort_layers(&[e_d, e_c, e_b, e_a]).expect("diamond should succeed");

    // L1 must be sorted by name: C=Gateway < B=Storage
    assert_eq!(layers[1], vec![Gateway, Storage]);
}

// --------------------------------------------------------------------------
// SpawnController and AdminRpcServer dependency validation
// --------------------------------------------------------------------------

#[test]
fn test_spawn_controller_depends_on_agent_registry() {
    use ComponentId::*;
    let entries = all_component_entries();
    let dep_map: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        entries.iter().map(|e| (e.id, e.deps.clone())).collect();

    assert_eq!(
        dep_map[&SpawnController],
        vec![AgentRegistry],
        "SpawnController must depend on AgentRegistry per design doc Layer 3"
    );
}

#[test]
fn test_admin_rpc_server_depends_on_gateway() {
    use ComponentId::*;
    let entries = all_component_entries();
    let dep_map: std::collections::HashMap<ComponentId, Vec<ComponentId>> =
        entries.iter().map(|e| (e.id, e.deps.clone())).collect();

    assert_eq!(
        dep_map[&AdminRpcServer],
        vec![Gateway],
        "AdminRpcServer must depend on Gateway per design doc Layer 5/6"
    );
}

#[test]
fn test_spawn_controller_in_core_services_layer() {
    use ComponentId::*;
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    // SpawnController is in Layer 3 (CoreServices phase)
    // Layer index 2 = third layer
    assert!(
        layers[2].contains(&SpawnController),
        "SpawnController must be in Layer 3 (CoreServices), got layers: {:?}",
        layers
            .iter()
            .enumerate()
            .map(|(i, l)| (i, l.iter().map(|c| c.name()).collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_admin_rpc_server_in_post_gateway_layer() {
    use ComponentId::*;
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    // AdminRpcServer is in Layer 6 (PostGateway phase)
    // Layer index 5 = sixth layer
    assert!(
        layers[5].contains(&AdminRpcServer),
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
    use ComponentId::*;
    // Manually build layers with SpawnController misplaced into Layer 1
    let wrong_layers: Vec<Vec<ComponentId>> = vec![
        vec![ConfigManager, Storage, SpawnController], // Wrong: SpawnController here
        vec![
            AgentRegistry,
            ConfigHotReload,
            RenderersPlugins,
            SessionConfigProvider,
            SkillsRegistry,
        ],
        vec![
            ArchiveSweeper,
            DreamingScheduler,
            IMAdapters,
            PermissionEngine,
            SkillWatcher,
            SystemPromptBuilder,
            ToolsRegistry,
        ],
        vec![ApprovalFlow, SessionManager],
        vec![Gateway],
        vec![AdminRpcServer],
    ];
    let err = validate_startup_layers(&wrong_layers).unwrap_err();
    assert!(
        matches!(err, StartupError::CircularDependency),
        "validation should reject wrong SpawnController layer placement"
    );
}

#[test]
fn test_validate_layers_catches_wrong_admin_rpc_server_layer() {
    use ComponentId::*;
    // Manually build layers with AdminRpcServer misplaced into Layer 4
    let wrong_layers: Vec<Vec<ComponentId>> = vec![
        vec![ConfigManager, Storage],
        vec![
            AgentRegistry,
            ConfigHotReload,
            RenderersPlugins,
            SessionConfigProvider,
            SkillsRegistry,
        ],
        vec![
            ArchiveSweeper,
            DreamingScheduler,
            IMAdapters,
            PermissionEngine,
            SkillWatcher,
            SpawnController,
            SystemPromptBuilder,
            ToolsRegistry,
        ],
        vec![ApprovalFlow, SessionManager, AdminRpcServer], // Wrong: AdminRpcServer here
        vec![Gateway],
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
    use ComponentId::*;
    // Three independent nodes → should all be in L0, sorted by name.
    let e_a = entry(AgentRegistry, "Zebra", vec![]);
    let e_b = entry(ConfigManager, "Apple", vec![]);
    let e_c = entry(Storage, "Mango", vec![]);

    let layers = topo_sort_layers(&[e_a, e_b, e_c]).expect("should succeed");
    // Three independent nodes → one layer, sorted by id.name()
    // AgentRegistry < ConfigManager < Storage
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0], vec![AgentRegistry, ConfigManager, Storage]);
}

// --------------------------------------------------------------------------
// Linear chain: A → B → C → D
// --------------------------------------------------------------------------

#[test]
fn test_linear_chain() {
    use ComponentId::*;
    // Each depends on the previous; should produce 4 layers.
    let e_d = entry(AgentRegistry, "D", vec![Storage]);
    let e_c = entry(Storage, "C", vec![Gateway]);
    let e_b = entry(Gateway, "B", vec![ConfigManager]);
    let e_a = entry(ConfigManager, "A", vec![]);

    let layers = topo_sort_layers(&[e_d, e_c, e_b, e_a]).expect("linear chain should succeed");

    assert_eq!(layers.len(), 4);
    assert_eq!(layers[0], vec![ConfigManager]);
    assert_eq!(layers[1], vec![Gateway]);
    assert_eq!(layers[2], vec![Storage]);
    assert_eq!(layers[3], vec![AgentRegistry]);
}

// --------------------------------------------------------------------------
// All nodes in parallel (no deps between any pair)
// --------------------------------------------------------------------------

#[test]
fn test_all_parallel() {
    use ComponentId::*;
    let entries = vec![
        entry(ConfigManager, "C", vec![]),
        entry(AgentRegistry, "A", vec![]),
        entry(Storage, "B", vec![]),
    ];
    let layers = topo_sort_layers(&entries).expect("all parallel should succeed");

    assert_eq!(layers.len(), 1);
    // A < B < C by id.name(): AgentRegistry < ConfigManager < Storage
    assert_eq!(layers[0], vec![AgentRegistry, ConfigManager, Storage]);
}
