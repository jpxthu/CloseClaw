//! init_disk_skills - initializes the disk skill registry at startup.

use super::{registry::DiskSkillRegistry, scan_all_skills};

/// Initializes the disk skill registry by scanning all configured skill directories.
///
/// Returns a [`DiskSkillRegistry`] containing all discovered disk skills,
/// or an empty registry if no skills are found or all scans fail.
pub fn init_disk_skills(config: &super::ScanConfig) -> DiskSkillRegistry {
    let skills = scan_all_skills(config);
    let loaded = skills.len();

    tracing::info!(
        loaded = loaded,
        skipped = 0,
        errors = 0,
        "disk skill scan complete",
    );

    DiskSkillRegistry::new(skills)
}

#[cfg(test)]
mod tests {
    use super::super::ScanConfig;

    #[test]
    fn test_init_disk_skills_empty_config() {
        let config = ScanConfig::default();
        let registry = super::init_disk_skills(&config);
        assert!(registry.is_empty());
    }

    #[test]
    fn test_init_disk_skills_nonexistent_dir() {
        let config = ScanConfig {
            bundled_dir: Some(std::path::PathBuf::from("/nonexistent")),
            ..Default::default()
        };
        let registry = super::init_disk_skills(&config);
        assert!(registry.is_empty());
    }

    #[test]
    fn test_init_disk_skills_with_valid_dir() {
        let temp = tempfile::tempdir().unwrap();
        let skill_dir = temp.path().join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: a test skill\n---\n# Test\n",
        )
        .unwrap();

        let config = ScanConfig {
            bundled_dir: Some(temp.path().to_path_buf()),
            ..Default::default()
        };
        let registry = super::init_disk_skills(&config);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.list(), vec!["test-skill"]);
    }
}
