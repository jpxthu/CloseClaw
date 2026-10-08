//! Composition-root assembly of the values injected into platform plugins.
//!
//! `im_adapter` no longer reads configuration files itself (issue #3347):
//! the daemon assembles the identity mapping (`config/accounts.json`), the
//! feishu profile (`config/credentials/`) and the media config
//! (`config/media.json`) here, then injects them through
//! `closeclaw_im_adapter::platforms::register_platform_plugins`.

use std::sync::Arc;

use closeclaw_common::identity::IdentityResolver;
use closeclaw_config::identity::ConfigIdentityResolver;
use closeclaw_config::{AccountsConfigData, CredentialsProvider, MediaConfigData};
use closeclaw_im_adapter::platforms::MediaConfigSnapshot;
use tracing::{info, warn};

/// Values assembled once per platform (re)registration and injected into
/// the platform plugins by the composition root.
pub(crate) struct PlatformInjection {
    /// Config-backed identity resolver — `None` when `accounts.json` is
    /// missing, empty or unparseable (fallback: `sender_id` as account).
    pub(crate) identity_resolver: Option<Arc<dyn IdentityResolver>>,
    /// Feishu profile from `config/credentials/` — `None` when absent
    /// (the adapter falls back to the `FEISHU_PROFILE` env var).
    pub(crate) feishu_profile: Option<String>,
    /// Media config — defaults when `media.json` is missing/unparseable.
    pub(crate) media_config: MediaConfigData,
}

impl PlatformInjection {
    /// Assemble every injected value for `config_dir`.
    pub(crate) fn load(config_dir: &str) -> Self {
        Self {
            identity_resolver: build_identity_resolver(config_dir),
            feishu_profile: build_feishu_profile(config_dir),
            media_config: load_media_config(config_dir),
        }
    }

    /// Project the media config onto the adapter's read-only snapshot.
    pub(crate) fn media_config_snapshot(&self) -> MediaConfigSnapshot {
        MediaConfigSnapshot {
            storage_dir: self.media_config.storage_dir.clone().into(),
            retention_days: self.media_config.retention_days,
            image_content_threshold_bytes: self.media_config.image_content_threshold_bytes,
        }
    }
}

/// Load `{config_dir}/config/accounts.json` and build the identity
/// resolver, or `None` on a missing/empty/unparseable file.
fn build_identity_resolver(config_dir: &str) -> Option<Arc<dyn IdentityResolver>> {
    let path = std::path::Path::new(config_dir)
        .join("config")
        .join("accounts.json");
    let json = match std::fs::read_to_string(&path) {
        Ok(json) => json,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!("accounts.json not found — identity mapping disabled");
            return None;
        }
        Err(e) => {
            warn!(
                error = %e,
                path = %path.display(),
                "failed to read accounts.json — skipping identity mapping"
            );
            return None;
        }
    };
    let accounts = match AccountsConfigData::from_json_str(&json) {
        Ok(accounts) => accounts,
        Err(e) => {
            warn!(
                error = %e,
                path = %path.display(),
                "failed to parse accounts.json — skipping identity mapping"
            );
            return None;
        }
    };
    let resolver = ConfigIdentityResolver::new(accounts.accounts);
    if resolver.is_empty() {
        info!("accounts.json loaded but empty — no mappings configured");
        return None;
    }
    info!(
        count = resolver.len(),
        "identity mapping loaded from {}",
        path.display()
    );
    Some(Arc::new(resolver))
}

/// Load the feishu profile from `{config_dir}/config/credentials/`, or
/// `None` when no feishu credential file is present.
fn build_feishu_profile(config_dir: &str) -> Option<String> {
    CredentialsProvider::load_from_dir(
        &std::path::Path::new(config_dir)
            .join("config")
            .join("credentials"),
    )
    .ok()
    .and_then(|creds| creds.feishu_profile().map(|p| p.profile.clone()))
}

/// Load `{config_dir}/config/media.json`, falling back to defaults when
/// the file is missing or unparseable.
fn load_media_config(config_dir: &str) -> MediaConfigData {
    let path = std::path::Path::new(config_dir)
        .join("config")
        .join("media.json");
    match MediaConfigData::from_file(&path) {
        Ok(cfg) => {
            info!(
                storage_dir = %cfg.storage_dir,
                "media config loaded from {}",
                path.display()
            );
            cfg
        }
        Err(e) => {
            warn!(
                error = %e,
                path = %path.display(),
                "failed to load media.json — using defaults"
            );
            MediaConfigData::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a temp `config_dir` optionally containing `accounts.json`.
    fn setup_config_dir(accounts_json: Option<&str>) -> TempDir {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config");
        fs::create_dir_all(&config).unwrap();
        if let Some(content) = accounts_json {
            fs::write(config.join("accounts.json"), content).unwrap();
        }
        dir
    }

    // ── identity resolver assembly ───────────────────────────────────

    /// accounts.json exists with valid mappings → resolver loaded with
    /// correct mappings.
    #[test]
    fn identity_resolver_loads_accounts_json() {
        let json = r#"{
            "accounts": [
                {"platform": "feishu", "sender_id": "ou_aaa", "account_id": "user1"},
                {"platform": "feishu", "sender_id": "ou_bbb", "account_id": "user2"}
            ]
        }"#;
        let dir = setup_config_dir(Some(json));
        let resolver = build_identity_resolver(dir.path().to_str().unwrap());
        assert!(resolver.is_some());
        let r = resolver.unwrap();
        assert_eq!(r.resolve("feishu", "", "ou_aaa"), Some("user1".into()));
        assert_eq!(r.resolve("feishu", "", "ou_bbb"), Some("user2".into()));
    }

    /// accounts.json missing → no resolver.
    #[test]
    fn identity_resolver_missing_file_returns_none() {
        let dir = setup_config_dir(None);
        assert!(build_identity_resolver(dir.path().to_str().unwrap()).is_none());
    }

    /// accounts.json empty → no resolver.
    #[test]
    fn identity_resolver_empty_accounts_returns_none() {
        let dir = setup_config_dir(Some(r#"{"accounts":[]}"#));
        assert!(build_identity_resolver(dir.path().to_str().unwrap()).is_none());
    }

    /// accounts.json with invalid JSON → no resolver.
    #[test]
    fn identity_resolver_invalid_json_returns_none() {
        let dir = setup_config_dir(Some("not json"));
        assert!(build_identity_resolver(dir.path().to_str().unwrap()).is_none());
    }

    /// Unknown sender → `None` (call site falls back to the raw sender_id).
    #[test]
    fn identity_resolver_unknown_sender_returns_none() {
        let json =
            r#"{"accounts":[{"platform":"feishu","sender_id":"ou_xxx","account_id":"user1"}]}"#;
        let dir = setup_config_dir(Some(json));
        let resolver = build_identity_resolver(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(resolver.resolve("feishu", "", "ou_unknown"), None);
    }

    /// Cross-platform isolation: a feishu mapping does not affect discord.
    #[test]
    fn identity_resolver_cross_platform_isolation() {
        let json = r#"{
            "accounts": [
                {"platform": "feishu", "sender_id": "ou_aaa", "account_id": "user1"},
                {"platform": "discord", "sender_id": "12345", "account_id": "user2"}
            ]
        }"#;
        let dir = setup_config_dir(Some(json));
        let resolver = build_identity_resolver(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(
            resolver.resolve("feishu", "", "ou_aaa"),
            Some("user1".into())
        );
        assert_eq!(
            resolver.resolve("discord", "", "12345"),
            Some("user2".into())
        );
        assert_eq!(resolver.resolve("discord", "", "ou_aaa"), None);
        assert_eq!(resolver.resolve("feishu", "", "12345"), None);
    }

    /// Several senders on different platforms may map to one account.
    #[test]
    fn identity_resolver_many_to_one() {
        let json = r#"{
            "accounts": [
                {"platform": "feishu", "sender_id": "ou_aaa", "account_id": "alice"},
                {"platform": "discord", "sender_id": "99", "account_id": "alice"},
                {"platform": "slack", "sender_id": "U001", "account_id": "alice"}
            ]
        }"#;
        let dir = setup_config_dir(Some(json));
        let resolver = build_identity_resolver(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(
            resolver.resolve("feishu", "", "ou_aaa"),
            Some("alice".into())
        );
        assert_eq!(resolver.resolve("discord", "", "99"), Some("alice".into()));
        assert_eq!(resolver.resolve("slack", "", "U001"), Some("alice".into()));
    }

    // ── feishu profile assembly ──────────────────────────────────────

    /// Feishu credential file present → injected profile is its profile.
    #[test]
    fn feishu_profile_loaded_from_credentials_dir() {
        let dir = setup_config_dir(None);
        let credentials = dir.path().join("config").join("credentials");
        fs::create_dir_all(&credentials).unwrap();
        fs::write(
            credentials.join("feishu.json"),
            r#"{"provider":"feishu","profile":"my_feishu_profile"}"#,
        )
        .unwrap();
        assert_eq!(
            build_feishu_profile(dir.path().to_str().unwrap()),
            Some("my_feishu_profile".to_string())
        );
    }

    /// No feishu credential file → `None` (adapter falls back to env).
    #[test]
    fn feishu_profile_missing_returns_none() {
        let dir = setup_config_dir(None);
        assert!(build_feishu_profile(dir.path().to_str().unwrap()).is_none());
    }

    // ── media config assembly + projection ───────────────────────────

    /// media.json missing → defaults (historical phase_init semantics).
    #[test]
    fn media_config_missing_file_uses_defaults() {
        let dir = setup_config_dir(None);
        assert_eq!(
            load_media_config(dir.path().to_str().unwrap()),
            MediaConfigData::default()
        );
    }

    /// media.json present → its values are loaded.
    #[test]
    fn media_config_loaded_from_file() {
        let dir = setup_config_dir(None);
        fs::write(
            dir.path().join("config").join("media.json"),
            r#"{
                "storageDir": "media-x",
                "retentionDays": 3,
                "imageContentThresholdBytes": 42
            }"#,
        )
        .unwrap();
        let cfg = load_media_config(dir.path().to_str().unwrap());
        assert_eq!(cfg.storage_dir, "media-x");
        assert_eq!(cfg.retention_days, 3);
        assert_eq!(cfg.image_content_threshold_bytes, 42);
    }

    /// Projection copies every field onto the adapter's snapshot.
    #[test]
    fn media_config_snapshot_projects_fields() {
        let injection = PlatformInjection {
            identity_resolver: None,
            feishu_profile: None,
            media_config: MediaConfigData {
                storage_dir: "media-y".to_string(),
                retention_days: 9,
                image_content_threshold_bytes: 7,
                ..Default::default()
            },
        };
        assert_eq!(
            injection.media_config_snapshot(),
            MediaConfigSnapshot {
                storage_dir: std::path::PathBuf::from("media-y"),
                retention_days: 9,
                image_content_threshold_bytes: 7,
            }
        );
    }

    /// The im_adapter default snapshot must stay in step with the config
    /// crate's media defaults (the two definitions cannot reference each
    /// other across the dependency boundary).
    #[test]
    fn media_config_snapshot_default_matches_config_default() {
        let injection = PlatformInjection {
            identity_resolver: None,
            feishu_profile: None,
            media_config: MediaConfigData::default(),
        };
        assert_eq!(
            injection.media_config_snapshot(),
            MediaConfigSnapshot::default()
        );
    }
}
