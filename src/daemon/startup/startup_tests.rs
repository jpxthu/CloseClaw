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
    assert_eq!(entries.len(), 17);
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
}

#[test]
fn test_topo_sort_five_layers_match_design_doc() {
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");

    assert_eq!(layers.len(), 5, "expected exactly 5 layers");

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
    //          PermissionEngine, SkillWatcher, SystemPromptBuilder, ToolsRegistry
    assert_eq!(
        layers[2],
        vec![
            ArchiveSweeper,
            DreamingScheduler,
            IMAdapters,
            PermissionEngine,
            SkillWatcher,
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
}
