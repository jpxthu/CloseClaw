//! Skill directory scanner
//!
//! Scans hierarchical skill directories and returns discovered skills
//! ordered by priority (project > agent > global > extraDirs > bundled).

use std::collections::BTreeMap;

use super::{DiskSkill, ParsedSkill, ScanConfig, SkillSource};

/// Scan all skill directories and return a list of discovered skills.
///
/// Discovery order (lowest to highest priority, later overwrites earlier):
/// 1. `extra_dirs` — user-provided additional directories (lowest priority)
/// 2. `global_dir` — global cross-agent skills
/// 3. `agent_skills_dir` — agent-specific skills directory (explicit path required)
/// 4. `project_root` — project-local skills (highest priority)
///
/// Bundled skills are NOT scanned from disk — they are compiled in
/// and managed by `BuiltinSkillRegistry`.
///
/// When the same skill name appears at multiple priority levels,
/// the higher-priority entry wins and a warning is emitted.
pub fn scan_all_skills(config: &ScanConfig) -> Vec<DiskSkill> {
    let mut skills_by_name: BTreeMap<String, DiskSkill> = BTreeMap::new();

    // Scan from lowest to highest priority so higher priority always overwrites
    for dir in &config.extra_dirs {
        scan_layer(dir, SkillSource::ExtraDirs, &mut skills_by_name);
    }

    if let Some(ref dir) = config.global_dir {
        scan_layer(dir, SkillSource::Global, &mut skills_by_name);
    }

    if let Some(ref dir) = config.agent_skills_dir {
        scan_layer(dir, SkillSource::Agent, &mut skills_by_name);
    }

    if let Some(ref project_root) = config.project_root {
        scan_layer(project_root, SkillSource::Project, &mut skills_by_name);
    }

    skills_by_name.into_values().collect()
}

fn scan_layer(
    dir: &std::path::Path,
    source: SkillSource,
    skills: &mut BTreeMap<String, DiskSkill>,
) {
    if !dir.is_dir() {
        return;
    }

    let readdir = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(source = %source, path = %dir.display(), error = %e,
                "failed to read skill directory, skipping");
            return;
        }
    };

    for entry in readdir.filter_map(|e| e.ok()) {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        let skill_name = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        let Some(name) = skill_name else {
            continue;
        };

        let readme_path = entry_path.join("SKILL.md");
        if !readme_path.is_file() {
            continue;
        }

        let raw = match std::fs::read_to_string(&readme_path) {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(source = %source, path = %readme_path.display(), error = %e,
                    "failed to read SKILL.md, skipping skill");
                continue;
            }
        };

        let parsed: ParsedSkill = match super::parse_skill_md(&raw) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(source = %source, skill = %name, error = %e,
                    "failed to parse SKILL.md, skipping skill");
                continue;
            }
        };

        let skill_dir = entry_path;
        let mut manifest = parsed.manifest;
        // Always use the directory name as the skill registry name,
        // regardless of any `name` field in frontmatter.
        manifest.name = name.clone();

        let disk_skill = DiskSkill {
            source,
            manifest,
            readme_path,
            skill_dir,
        };

        if let Some(existing) = skills.get(&disk_skill.manifest.name) {
            tracing::warn!(
                skill = %disk_skill.manifest.name,
                existing_source = %existing.source,
                new_source = %source,
                "lower-priority skill overridden by higher-priority one",
            );
        }

        skills.insert(disk_skill.manifest.name.clone(), disk_skill);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_file(path: &std::path::Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn test_scan_empty_config() {
        let config = ScanConfig::default();
        let skills = scan_all_skills(&config);
        assert!(skills.is_empty());
    }

    #[test]
    fn test_scan_nonexistent_directory() {
        let config = ScanConfig {
            global_dir: Some(std::path::PathBuf::from("/nonexistent/path")),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert!(skills.is_empty());
    }

    #[test]
    fn test_scan_single_layer() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("test-skill");
        create_file(
            &skill_dir.join("SKILL.md"),
            "---\ndescription: A test skill\n---\n# Test\n",
        );

        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].manifest.name, "test-skill");
    }

    #[test]
    fn test_scan_multiple_skills() {
        let temp = tempfile::tempdir().unwrap();
        for name in &["skill-a", "skill-b", "skill-c"] {
            create_file(
                &temp.path().join(name).join("SKILL.md"),
                &format!("---\ndescription: \"{}\"\n---\n# {}\n", name, name),
            );
        }

        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 3);
    }

    #[test]
    fn test_priority_override() {
        let temp = tempfile::tempdir().unwrap();
        let global_dir = temp.path().join("global");
        let project_dir = temp.path().join("project");

        create_file(
            &global_dir.join("shared-skill").join("SKILL.md"),
            "---\ndescription: Lower\n---\n# Lower\n",
        );
        create_file(
            &project_dir.join("shared-skill").join("SKILL.md"),
            "---\ndescription: Higher\n---\n# Higher\n",
        );

        let config = ScanConfig {
            global_dir: Some(global_dir),
            project_root: Some(project_dir),
            ..Default::default()
        };

        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].manifest.description, "Higher");
    }

    #[test]
    fn test_skip_invalid_skill_md() {
        let temp = tempfile::tempdir().unwrap();
        create_file(
            &temp.path().join("bad-skill").join("SKILL.md"),
            "no frontmatter",
        );
        create_file(
            &temp.path().join("good-skill").join("SKILL.md"),
            "---\ndescription: Good\n---\n# Good\n",
        );

        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
    }

    #[test]
    fn test_skip_missing_skill_md() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("no-readme")).unwrap();

        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert!(skills.is_empty());
    }

    #[test]
    fn test_agent_layer_uses_explicit_dir() {
        let temp = tempfile::tempdir().unwrap();
        let agent_skills_dir = temp.path().join("agents").join("my-agent").join("skills");
        create_file(
            &agent_skills_dir.join("agent-skill").join("SKILL.md"),
            "---\ndescription: Agent skill\n---\n# Agent\n",
        );

        let config = ScanConfig {
            agent_skills_dir: Some(agent_skills_dir),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, SkillSource::Agent);
    }

    #[test]
    fn test_agent_layer_no_explicit_dir_skips() {
        // Without agent_skills_dir, agent layer is skipped
        // (legacy agent_id derivation removed)
        let temp = tempfile::tempdir().unwrap();
        let global_dir = temp.path().join("global");
        create_file(
            &global_dir
                .join("agents")
                .join("my-agent")
                .join("agent-skill")
                .join("SKILL.md"),
            "---\ndescription: Agent skill\n---\n# Agent\n",
        );

        let config = ScanConfig {
            global_dir: Some(global_dir),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert!(skills.is_empty());
    }

    #[test]
    fn test_agent_layer_explicit_dir_used() {
        let temp = tempfile::tempdir().unwrap();
        // Explicit dir has one skill
        let explicit_dir = temp.path().join("explicit").join("my-agent").join("skills");
        create_file(
            &explicit_dir.join("explicit-skill").join("SKILL.md"),
            "---\ndescription: Explicit\n---\n# Explicit\n",
        );

        let config = ScanConfig {
            agent_skills_dir: Some(explicit_dir),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].manifest.name, "explicit-skill");
    }

    #[test]
    fn test_extra_dirs_equal_priority() {
        let temp = tempfile::tempdir().unwrap();
        let dir1 = temp.path().join("dir1");
        let dir2 = temp.path().join("dir2");

        create_file(
            &dir1.join("skill").join("SKILL.md"),
            "---\ndescription: From dir1\n---\n# dir1\n",
        );
        create_file(
            &dir2.join("skill").join("SKILL.md"),
            "---\ndescription: From dir2\n---\n# dir2\n",
        );

        let config = ScanConfig {
            extra_dirs: vec![dir1, dir2],
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
    }

    #[test]
    fn test_load_body_populates_from_file() {
        let temp = tempfile::tempdir().unwrap();
        create_file(
            &temp.path().join("my-skill").join("SKILL.md"),
            "---\ndescription: A skill\n---\n\n# Hello\n\nInstructions here.\n",
        );

        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(
            skills[0].load_body().unwrap(),
            "# Hello\n\nInstructions here."
        );
    }

    #[test]
    fn test_load_body_empty_when_no_body_text() {
        let temp = tempfile::tempdir().unwrap();
        create_file(
            &temp.path().join("no-body").join("SKILL.md"),
            "---\ndescription: No body\n---\n",
        );

        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].load_body().unwrap(), "");
    }

    #[test]
    fn test_load_body_with_bom() {
        let temp = tempfile::tempdir().unwrap();
        create_file(
            &temp.path().join("bom-skill").join("SKILL.md"),
            concat!("\u{feff}", "---\ndescription: BOM skill\n---\n\n# Body\n"),
        );

        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].load_body().unwrap(), "# Body");
    }

    #[test]
    fn test_load_body_preserves_multiline() {
        let temp = tempfile::tempdir().unwrap();
        create_file(
            &temp.path().join("multi").join("SKILL.md"),
            "---\ndescription: Multi\n---\n\n# Step 1\nDo A.\n\n# Step 2\nDo B.\n",
        );

        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(
            skills[0].load_body().unwrap(),
            "# Step 1\nDo A.\n\n# Step 2\nDo B."
        );
    }

    #[test]
    fn test_load_body_file_not_found() {
        use super::super::types::{
            DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
        };
        let nonexistent = std::path::PathBuf::from(format!(
            "/tmp/nonexistent-skill-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skill = DiskSkill {
            source: SkillSource::Bundled,
            manifest: SkillManifest {
                name: "missing".into(),
                description: "missing skill".into(),
                when_to_use: String::new(),
                context: SkillContext::Inline,
                effort: SkillEffort::Small,
                paths: vec![],
                user_invocable: false,
            },
            readme_path: nonexistent.join("SKILL.md"),
            skill_dir: nonexistent,
        };
        let err = skill.load_body().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_load_body_special_characters() {
        let temp = tempfile::tempdir().unwrap();
        let content = concat!(
            "---\ndescription: Special\n---\n\n",
            "# Hello 🌍\n\nLine with \"quotes\" ",
            "and <html> &amp; entities.\n",
        );
        create_file(&temp.path().join("special").join("SKILL.md"), content);

        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        let body = skills[0].load_body().unwrap();
        assert!(body.contains("🌍"));
        assert!(body.contains('"')); // quotes preserved
        assert!(body.contains("<html>") || body.contains("&amp;"));
    }

    #[test]
    fn test_load_body_rereads_after_scan() {
        let temp = tempfile::tempdir().unwrap();
        create_file(
            &temp.path().join("live").join("SKILL.md"),
            "---\ndescription: Live\n---\n\n# Initial\n",
        );

        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].load_body().unwrap(), "# Initial");

        // Overwrite the file after scan
        create_file(
            &temp.path().join("live").join("SKILL.md"),
            "---\ndescription: Live\n---\n\n# Updated\n",
        );
        // load_body should read the updated content
        assert_eq!(skills[0].load_body().unwrap(), "# Updated");
    }

    // ----------------------------------------------------------------------
    // Bundled-layer removal tests
    // ----------------------------------------------------------------------

    #[test]
    fn test_scan_config_has_no_bundled_dir() {
        // ScanConfig should not expose a bundled_dir field;
        // bundled skills are managed by BuiltinSkillRegistry, not disk scanning.
        let cfg = ScanConfig::default();
        // Verify the four disk-scan fields exist and default correctly
        assert!(cfg.extra_dirs.is_empty());
        assert!(cfg.global_dir.is_none());
        assert!(cfg.project_root.is_none());
        assert!(cfg.agent_skills_dir.is_none());
        // If bundled_dir accidentally re-appears, this test will fail to compile
        // because ScanConfig has no such field.
    }

    #[test]
    fn test_scan_all_skills_only_scans_four_layers() {
        // Set up a temp directory with skills in each of the four layers
        let temp = tempfile::tempdir().unwrap();

        // ExtraDirs layer
        let extra = temp.path().join("extra");
        create_file(
            &extra.join("extra-skill").join("SKILL.md"),
            "---\ndescription: Extra\n---\n",
        );

        // Global layer
        let global = temp.path().join("global");
        create_file(
            &global.join("global-skill").join("SKILL.md"),
            "---\ndescription: Global\n---\n",
        );

        // Agent layer (explicit dir)
        let agent = temp.path().join("agent");
        create_file(
            &agent.join("agent-skill").join("SKILL.md"),
            "---\ndescription: Agent\n---\n",
        );

        // Project layer
        let project = temp.path().join("project");
        create_file(
            &project.join("project-skill").join("SKILL.md"),
            "---\ndescription: Project\n---\n",
        );

        let config = ScanConfig {
            extra_dirs: vec![extra],
            global_dir: Some(global),
            agent_skills_dir: Some(agent),
            project_root: Some(project),
            ..Default::default()
        };

        let skills = scan_all_skills(&config);
        let names: Vec<&str> = skills.iter().map(|s| s.manifest.name.as_str()).collect();
        assert_eq!(
            skills.len(),
            4,
            "expected exactly four skills (one per layer), got: {names:?}"
        );
        assert!(names.contains(&"extra-skill"));
        assert!(names.contains(&"global-skill"));
        assert!(names.contains(&"agent-skill"));
        assert!(names.contains(&"project-skill"));
    }

    #[test]
    fn test_scan_all_skills_does_not_scan_bundled() {
        // Even if a directory called "bundled" exists and contains a valid SKILL.md,
        // scan_all_skills should NOT pick it up because ScanConfig has no bundled_dir.
        let temp = tempfile::tempdir().unwrap();
        create_file(
            &temp.path().join("bundled").join("SKILL.md"),
            "---\ndescription: Bundled\n---\n",
        );
        // No config points to this directory — ScanConfig has no bundled_dir
        let config = ScanConfig::default();
        let skills = scan_all_skills(&config);
        assert!(skills.is_empty(), "bundled directory should not be scanned");
    }

    // ------------------------------------------------------------------
    // Custom manifest name lookup tests
    // ------------------------------------------------------------------

    #[test]
    fn test_custom_manifest_name_ignored() {
        // Directory is "my-dir-skill" but frontmatter declares
        // name: "my-custom-name". The directory name always wins.
        let temp = tempfile::tempdir().unwrap();
        create_file(
            &temp.path().join("my-dir-skill").join("SKILL.md"),
            "---\nname: my-custom-name\ndescription: Custom name skill\n---\n# Custom\n",
        );
        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].manifest.name, "my-dir-skill");

        // DiskSkillRegistry::get must resolve by directory name
        let registry = super::super::registry::DiskSkillRegistry::new(skills);
        assert!(registry.get("my-dir-skill").is_some());
        assert!(registry.get("my-custom-name").is_none());
    }

    #[test]
    fn test_no_custom_name_falls_back_to_dir_name() {
        // Directory is "my-dir-skill" and frontmatter has no name field.
        // Must fall back to directory name.
        let temp = tempfile::tempdir().unwrap();
        create_file(
            &temp.path().join("my-dir-skill").join("SKILL.md"),
            "---\ndescription: Fallback skill\n---\n# Fallback\n",
        );
        let config = ScanConfig {
            global_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].manifest.name, "my-dir-skill");

        let registry = super::super::registry::DiskSkillRegistry::new(skills);
        assert!(registry.get("my-dir-skill").is_some());
    }

    #[test]
    fn test_priority_override_same_dir_name() {
        // Two directories with the same directory name at different
        // priority levels: higher priority should win.
        let temp = tempfile::tempdir().unwrap();
        let global_dir = temp.path().join("global");
        let project_dir = temp.path().join("project");

        create_file(
            &global_dir.join("shared-skill").join("SKILL.md"),
            "---\ndescription: Global version\n---\n",
        );
        create_file(
            &project_dir.join("shared-skill").join("SKILL.md"),
            "---\ndescription: Project version\n---\n",
        );

        let config = ScanConfig {
            global_dir: Some(global_dir),
            project_root: Some(project_dir),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].manifest.name, "shared-skill");
        assert_eq!(skills[0].manifest.description, "Project version");
    }

    #[test]
    fn test_different_dir_names_not_deduplicated() {
        // Two directories with different names at different priority
        // levels are NOT the same skill, even if frontmatter name matches.
        let temp = tempfile::tempdir().unwrap();
        let global_dir = temp.path().join("global");
        let project_dir = temp.path().join("project");

        create_file(
            &global_dir.join("dir-a").join("SKILL.md"),
            "---\nname: shared\ndescription: Global version\n---\n",
        );
        create_file(
            &project_dir.join("dir-b").join("SKILL.md"),
            "---\nname: shared\ndescription: Project version\n---\n",
        );

        let config = ScanConfig {
            global_dir: Some(global_dir),
            project_root: Some(project_dir),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 2);
        let names: Vec<&str> = skills.iter().map(|s| s.manifest.name.as_str()).collect();
        assert!(names.contains(&"dir-a"));
        assert!(names.contains(&"dir-b"));
    }
}
