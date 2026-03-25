//! Backup Manager — creates backups before writing, supports rollback
//!
//! Maintains a rolling history of the last N backups for each config file.

use chrono::Local;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// BackupManager provides backup/rollback functionality for config files.
///
/// It maintains a configurable number of rolling backups and allows
/// rolling back to the last known good state on error.
#[derive(Debug)]
pub struct BackupManager {
    /// Base directory for storing backups
    backup_dir: PathBuf,
    /// Maximum number of backups to retain per file
    max_backups: usize,
}

impl BackupManager {
    /// Create a new BackupManager with the given backup directory.
    /// The directory will be created if it doesn't exist.
    pub fn new<P: AsRef<Path>>(backup_dir: P, max_backups: usize) -> io::Result<Self> {
        let backup_dir = backup_dir.as_ref().to_path_buf();
        fs::create_dir_all(&backup_dir)?;
        Ok(Self {
            backup_dir,
            max_backups,
        })
    }

    /// Create a backup of the given file.
    /// Returns the path to the backup file.
    pub fn backup<P: AsRef<Path>>(&self, file_path: P) -> io::Result<PathBuf> {
        let file_path = file_path.as_ref();
        let content = fs::read(file_path)?;

        let timestamp = Local::now().format("%Y%m%d_%H%M%S_%f");
        let stem = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("config");
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("json");

        let backup_name = format!("{}.{}.{}", stem, timestamp, ext);
        let backup_path = self.backup_dir.join(&backup_name);

        fs::write(&backup_path, content)?;
        self.rotate_backups(file_path)?;

        Ok(backup_path)
    }

    /// Create a backup with the current content explicitly provided.
    /// Useful when you want to backup content before writing.
    pub fn backup_with_content<P: AsRef<Path>>(
        &self,
        file_path: P,
        content: &[u8],
    ) -> io::Result<PathBuf> {
        let file_path = file_path.as_ref();
        let timestamp = Local::now().format("%Y%m%d_%H%M%S_%f");
        let stem = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("config");
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("json");

        let backup_name = format!("{}.{}.{}", stem, timestamp, ext);
        let backup_path = self.backup_dir.join(&backup_name);

        fs::write(&backup_path, content)?;
        self.rotate_backups(file_path)?;

        Ok(backup_path)
    }

    /// Rollback to the most recent backup of the given file.
    /// Returns the path to the restored file.
    pub fn rollback<P: AsRef<Path>>(&self, file_path: P) -> io::Result<PathBuf> {
        let file_path = file_path.as_ref();
        let backup_path = self.find_latest_backup(file_path)?;

        if !backup_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("No backup found for {:?}", file_path),
            ));
        }

        let content = fs::read(&backup_path)?;
        fs::write(file_path, content)?;

        Ok(file_path.to_path_buf())
    }

    /// List all backups for a given file, newest first.
    pub fn list_backups<P: AsRef<Path>>(&self, file_path: P) -> io::Result<Vec<PathBuf>> {
        let file_path = file_path.as_ref();
        let stem = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("config");

        let mut backups: Vec<PathBuf> = fs::read_dir(&self.backup_dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    name.starts_with(stem) && name.contains(".")
                } else {
                    false
                }
            })
            .collect();

        backups.sort_by(|a, b| b.cmp(a)); // newest first
        Ok(backups)
    }

    /// Get the most recent backup path for a file.
    pub fn find_latest_backup<P: AsRef<Path>>(&self, file_path: P) -> io::Result<PathBuf> {
        let backups = self.list_backups(file_path)?;
        backups
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No backup found"))
    }

    /// Remove old backups beyond max_backups limit.
    fn rotate_backups<P: AsRef<Path>>(&self, file_path: P) -> io::Result<()> {
        let file_path = file_path.as_ref();
        let stem = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("config");

        let backups: Vec<PathBuf> = fs::read_dir(&self.backup_dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    name.starts_with(stem)
                } else {
                    false
                }
            })
            .collect();

        if backups.len() > self.max_backups {
            // Sort oldest first
            let mut sorted = backups;
            sorted.sort();
            // Remove oldest backups to get down to max_backups
            let to_remove = sorted.len() - self.max_backups;
            for path in sorted.into_iter().take(to_remove) {
                let _ = fs::remove_file(path);
            }
        }
        Ok(())
    }
}

/// Thread-safe wrapper for BackupManager using Mutex.
#[derive(Debug)]
pub struct SafeBackupManager(Mutex<BackupManager>);

impl SafeBackupManager {
    /// Create a new safe wrapper.
    pub fn new(manager: BackupManager) -> Self {
        Self(Mutex::new(manager))
    }

    /// Create a backup within a locked context.
    pub fn backup<P: AsRef<Path>>(&self, file_path: P) -> io::Result<PathBuf> {
        self.0.lock().unwrap().backup(file_path)
    }

    /// Create a backup with explicit content within a locked context.
    pub fn backup_with_content<P: AsRef<Path>>(
        &self,
        file_path: P,
        content: &[u8],
    ) -> io::Result<PathBuf> {
        self.0
            .lock()
            .unwrap()
            .backup_with_content(file_path, content)
    }

    /// Rollback within a locked context.
    pub fn rollback<P: AsRef<Path>>(&self, file_path: P) -> io::Result<PathBuf> {
        self.0.lock().unwrap().rollback(file_path)
    }

    /// List backups within a locked context.
    pub fn list_backups<P: AsRef<Path>>(&self, file_path: P) -> io::Result<Vec<PathBuf>> {
        self.0.lock().unwrap().list_backups(file_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[test]
    fn test_backup_and_restore() {
        let tmp_dir = TempDir::new().unwrap();
        let backup_dir = tmp_dir.path().join("backups");
        let manager = BackupManager::new(&backup_dir, 3).unwrap();

        let config_file = tmp_dir.path().join("agents.json");
        fs::write(&config_file, r#"{"version": "1.0.0", "agents": []}"#).unwrap();

        // Backup current state
        let backup_path = manager.backup(&config_file).unwrap();
        assert!(backup_path.exists());

        // Modify the file
        fs::write(&config_file, "modified content").unwrap();

        // Rollback
        manager.rollback(&config_file).unwrap();
        let content = fs::read_to_string(&config_file).unwrap();
        assert!(content.contains("1.0.0"));
    }

    #[test]
    fn test_max_backups_rotation() {
        let tmp_dir = TempDir::new().unwrap();
        let backup_dir = tmp_dir.path().join("backups");
        let manager = BackupManager::new(&backup_dir, 2).unwrap();

        let config_file = tmp_dir.path().join("test.json");
        fs::write(&config_file, "v1").unwrap();

        // Create 4 backups (more than max_backups=2)
        manager.backup(&config_file).unwrap();
        fs::write(&config_file, "v2").unwrap();
        manager.backup(&config_file).unwrap();
        fs::write(&config_file, "v3").unwrap();
        manager.backup(&config_file).unwrap();
        fs::write(&config_file, "v4").unwrap();
        manager.backup(&config_file).unwrap();

        let backups = manager.list_backups(&config_file).unwrap();
        assert_eq!(backups.len(), 2);
    }
}
