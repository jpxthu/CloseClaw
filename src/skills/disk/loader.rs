//! Skill directory scanner
//!
//! Scans hierarchical skill directories and returns discovered skills
//! ordered by priority (bundled > extraDirs > global > agent > project).

use std::collections::BTreeMap;

use super::{DiskSkill, ParsedSkill, ScanConfig, SkillSource};

/// Scan all skill directories and return a list of discovered skills.
///
/// Discovery order (highest to lowest priority):
/// 1. `bundled_dir` — built-in framework skills
/// 2. `extra_dirs` — user-provided additional directories
/// 3. `global_dir` — global cross-agent skills
/// 4. Agent-specific directory derived from `agent_id`
/// 5. `project_root` — project-local skills
///
/// When the same skill name appears at multiple priority levels,
/// the higher-priority entry wins and a warning is emitted.
pub fn scan_all_skills(config: &ScanConfig) -> Vec<DiskSkill> {
    let mut skills_by_name: BTreeMap<String, DiskSkill> = BTreeMap::new();

    // Scan from lowest to highest priority so higher priority always overwrites
    if let Some(ref project_root) = config.project_root {
        scan_layer(project_root, SkillSource::Project, &mut skills_by_name);
    }

    if let Some(ref agent_id) = config.agent_id {
        if let Some(ref global_dir) = config.global_dir {
            let agent_dir = global_dir.join("agents").join(agent_id);
            scan_layer(&agent_dir, SkillSource::Agent, &mut skills_by_name);
        }
    }

    if let Some(ref dir) = config.global_dir {
        scan_layer(dir, SkillSource::Global, &mut skills_by_name);
    }

    for dir in &config.extra_dirs {
        scan_layer(dir, SkillSource::ExtraDirs, &mut skills_by_name);
    }

    if let Some(ref dir) = config.bundled_dir {
        scan_layer(dir, SkillSource::Bundled, &mut skills_by_name);
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
        let manifest = if parsed.manifest.name.is_empty() {
            let mut m = parsed.manifest;
            m.name = name.clone();
            m
        } else {
            parsed.manifest
        };

        let disk_skill = DiskSkill {
            source,
            manifest,
            readme_path,
            skill_dir,
        };

        if let Some(existing) = skills.get(&name) {
            tracing::warn!(
                skill = %name,
                existing_source = %existing.source,
                new_source = %source,
                "lower-priority skill overridden by higher-priority one",
            );
        }

        skills.insert(name, disk_skill);
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
            bundled_dir: Some(std::path::PathBuf::from("/nonexistent/path")),
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
            bundled_dir: Some(temp.path().to_path_buf()),
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
            bundled_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 3);
    }

    #[test]
    fn test_priority_override() {
        let temp = tempfile::tempdir().unwrap();
        let lower_dir = temp.path().join("lower");
        let higher_dir = temp.path().join("higher");

        create_file(
            &lower_dir.join("shared-skill").join("SKILL.md"),
            "---\ndescription: Lower\n---\n# Lower\n",
        );
        create_file(
            &higher_dir.join("shared-skill").join("SKILL.md"),
            "---\ndescription: Higher\n---\n# Higher\n",
        );

        let config = ScanConfig {
            bundled_dir: Some(higher_dir),
            project_root: Some(lower_dir),
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
            bundled_dir: Some(temp.path().to_path_buf()),
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
            bundled_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert!(skills.is_empty());
    }

    #[test]
    fn test_agent_layer_derivation() {
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
            agent_id: Some("my-agent".to_string()),
            ..Default::default()
        };
        let skills = scan_all_skills(&config);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, SkillSource::Agent);
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
}
