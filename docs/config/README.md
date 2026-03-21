# Config Module

> Hot-reloadable JSON configuration system.

## Components

### ConfigProvider Trait
```rust
pub trait ConfigProvider {
    fn version(&self) -> &'static str;
    fn validate(&self) -> Result<(), ConfigError>;
    fn config_path() -> &'static str;
    fn is_default(&self) -> bool;
}
```

### AgentsConfigProvider
Loads and validates `agents.json`.

### ConfigReloadManager
Watches config files for changes and reloads automatically.

### BackupManager
Creates backups before writes, supports rollback.

## Config Files
- `agents.json` - Agent definitions
- `permissions.json` - Permission rules
- `skills.json` - Skill registry

## Hot Reload
Files watched: `agents.json`, `skills.json`
Not hot-reloaded: `permissions.json` (compile-time binding)

## Backup
- Location: `.closeclaw/backups/`
- Format: `{filename}.{timestamp}.bak`
- Max backups: 10 per file
