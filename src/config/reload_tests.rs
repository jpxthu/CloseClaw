#[cfg(test)]
mod reload_tests {
    use super::*;
    use crate::config::backup::{BackupManager, SafeBackupManager};
    use tempfile::TempDir;

    #[derive(Debug, Clone)]
    struct Cfg(String);
    impl ConfigProvider for Cfg {
        fn version(&self) -> &'static str {
            "1"
        }
        fn config_path() -> &'static str
        where
            Self: Sized,
        {
            "t"
        }
        fn is_default(&self) -> bool {
            self.0.is_empty()
        }
        fn validate(&self) -> Result<(), ConfigError> {
            if self.0.is_empty() {
                Err(ConfigError::ValueError {
                    field: "v".into(),
                    message: "empty".into(),
                })
            } else {
                Ok(())
            }
        }
    }

    fn mgr(dir: &std::path::Path, val: &str) -> ConfigReloadManager<Cfg> {
        let bm = BackupManager::new(dir.join("bak"), 3).unwrap();
        ConfigReloadManager::new(
            Cfg(val.into()),
            SafeBackupManager::new(bm),
            Duration::from_secs(1),
            |s: &str| {
                serde_json::from_str::<serde_json::Value>(s)
                    .map(|v| Cfg(v["v"].as_str().unwrap_or("").into()))
                    .map_err(|e| ConfigError::ValueError {
                        field: "p".into(),
                        message: e.to_string(),
                    })
            },
        )
    }

    #[test]
    fn test_new_and_snapshot() {
        let d = TempDir::new().unwrap();
        assert_eq!(mgr(d.path(), "hi").snapshot().0, "hi");
    }

    #[test]
    fn test_reload_success() {
        let d = TempDir::new().unwrap();
        let p = d.path().join("c.json");
        fs::write(&p, r#"{"v":"new"}"#).unwrap();
        let m = mgr(d.path(), "old");
        assert!(matches!(
            m.reload(&p.display().to_string()),
            ReloadResult::Success
        ));
        assert_eq!(m.snapshot().0, "new");
    }

    #[test]
    fn test_reload_validation_failed() {
        let d = TempDir::new().unwrap();
        let p = d.path().join("c.json");
        fs::write(&p, r#"{"v":""}"#).unwrap();
        let m = mgr(d.path(), "old");
        assert!(matches!(
            m.reload(&p.display().to_string()),
            ReloadResult::ValidationFailed(_)
        ));
        assert_eq!(m.snapshot().0, "old");
    }

    #[test]
    fn test_reload_nonexistent() {
        let d = TempDir::new().unwrap();
        assert!(matches!(
            mgr(d.path(), "x").reload("/no/file"),
            ReloadResult::RolledBack(_)
        ));
    }

    #[test]
    fn test_reload_invalid_json() {
        let d = TempDir::new().unwrap();
        let p = d.path().join("c.json");
        fs::write(&p, "bad").unwrap();
        assert!(matches!(
            mgr(d.path(), "x").reload(&p.display().to_string()),
            ReloadResult::ValidationFailed(_)
        ));
    }

    #[test]
    fn test_watcher_handle_stop() {
        let d = TempDir::new().unwrap();
        let p = d.path().join("c.json");
        fs::write(&p, r#"{"v":"x"}"#).unwrap();
        let mut m = mgr(d.path(), "old");
        m.watch(vec![p]).unwrap();
    }
}
