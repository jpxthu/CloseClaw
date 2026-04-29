//! Bootstrap File Loader — 架构唯一入口
//!
//! ⚠️ **架构唯一入口**：此模块中的 `load_bootstrap_files` 是工程中 bootstrap 文件加载的**唯一入口**。
//!
//! 后续任何直接读取 bootstrap 文件的代码（如 `fs::read_to_string(...AGENTS.md)`、
//! `Path::new(...).exists().read()` 等）均属架构违规，须在 code review 和架构 review 时
//! 即时发现并重构。此注释作为架构门禁，用于 code review 和架构 review 检查清单。
//!
//! # Bootstrap 文件集合
//!
//! | 文件       | Minimal | Full |
//! |------------|---------|------|
//! | AGENTS.md  | ✅      | ✅   |
//! | SOUL.md    | ✅      | ✅   |
//! | IDENTITY.md| ✅      | ✅   |
//! | USER.md    | ✅      | ✅   |
//! | TOOLS.md   | ✅      | ✅   |
//! | BOOTSTRAP.md| ❌     | ✅   |
//! | MEMORY.md  | ❌      | ✅   |
//! | HEARTBEAT.md| ❌     | ❌（不加载，由 agent 按需读取）|
//!
//! # HEARTBEAT.md 不属于 bootstrap 的原因
//!
//! HEARTBEAT.md 是 cron 触发 agent 时由 cron prompt 指示按需读取的动态上下文，
//! 不属于任何 session 启动时的固定 bootstrap 集合。将其纳入 bootstrap 会导致
//! 每次 cron 触发都加载不必要的 heartbeat 内容，增加 token 消耗且语义不正确。

use std::collections::HashMap;
use std::io;
use std::path::Path;

/// Bootstrap 文件集合模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapMode {
    /// 运行时必须的身份/工具文件，token 消耗最小
    Minimal,
    /// 完整集合，包含需要持久化的上下文/记忆文件
    Full,
}

/// Bootstrap 加载器错误类型
#[derive(Debug)]
pub enum BootstrapLoaderError {
    /// workspace_dir 无效或不存在
    InvalidWorkspace,
    /// IO 错误（读取文件失败）
    IoError(io::Error),
}

impl std::fmt::Display for BootstrapLoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootstrapLoaderError::InvalidWorkspace => {
                write!(f, "invalid workspace directory")
            }
            BootstrapLoaderError::IoError(e) => {
                write!(f, "IO error: {}", e)
            }
        }
    }
}

impl std::error::Error for BootstrapLoaderError {}

impl From<io::Error> for BootstrapLoaderError {
    fn from(e: io::Error) -> Self {
        BootstrapLoaderError::IoError(e)
    }
}

/// 列出给定 mode 下所有应加载的文件名（按 prepend 顺序）
pub fn bootstrap_file_list(mode: BootstrapMode) -> Vec<&'static str> {
    match mode {
        BootstrapMode::Minimal => {
            vec!["AGENTS.md", "SOUL.md", "IDENTITY.md", "USER.md", "TOOLS.md"]
        }
        BootstrapMode::Full => {
            vec![
                "AGENTS.md",
                "SOUL.md",
                "IDENTITY.md",
                "USER.md",
                "TOOLS.md",
                "BOOTSTRAP.md",
                "MEMORY.md",
            ]
        }
    }
}

/// ⚠️ 架构唯一入口：此函数是工程中 bootstrap 文件加载的唯一入口。
///
/// 后续任何直接读取 bootstrap 文件的代码（fs::read_to_string、Path::new(...).exists().read() 等）
/// 均属架构违规。此注释作为架构门禁，用于 code review 和架构 review 检查清单。
///
/// 加载 bootstrap 文件并返回 (filename → content) HashMap
///
/// # 文件集合
///
/// | 文件       | Minimal | Full |
/// |------------|---------|------|
/// | AGENTS.md  | ✅      | ✅   |
/// | SOUL.md    | ✅      | ✅   |
/// | IDENTITY.md| ✅      | ✅   |
/// | USER.md    | ✅      | ✅   |
/// | TOOLS.md   | ✅      | ✅   |
/// | BOOTSTRAP.md| ❌     | ✅   |
/// | MEMORY.md  | ❌      | ✅   |
/// | HEARTBEAT.md| ❌     | ❌（不加载，由 agent 按需读取）|
///
/// # HEARTBEAT.md 不属于 bootstrap 的原因
///
/// HEARTBEAT.md 是 cron 触发 agent 时由 cron prompt 指示按需读取的动态上下文，
/// 不属于任何 session 启动时的固定 bootstrap 集合。将其纳入 bootstrap 会导致
/// 每次 cron 触发都加载不必要的 heartbeat 内容，增加 token 消耗且语义不正确。
///
/// # 错误处理
///
/// - 文件不存在 → **跳过**（不返回 error），因为某些 workspace 可能没有全部文件
/// - workspace_dir 无效 → 返回 BootstrapLoaderError::InvalidWorkspace
pub fn load_bootstrap_files(
    workspace_dir: &Path,
    mode: BootstrapMode,
) -> Result<HashMap<String, String>, BootstrapLoaderError> {
    if !workspace_dir.is_dir() {
        return Err(BootstrapLoaderError::InvalidWorkspace);
    }

    let file_names = bootstrap_file_list(mode);
    let mut result = HashMap::new();

    for file_name in file_names {
        let file_path = workspace_dir.join(file_name);
        if file_path.is_file() {
            match std::fs::read_to_string(&file_path) {
                Ok(content) => {
                    result.insert(file_name.to_string(), content);
                }
                Err(e) => {
                    tracing::debug!(
                        file = file_name,
                        error = %e,
                        "failed to read bootstrap file, skipping"
                    );
                }
            }
        }
        // Non-existent files are skipped silently
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_files(dir: &Path, files: &[&str]) {
        for file_name in files {
            std::fs::write(dir.join(file_name), format!("content of {}", file_name)).unwrap();
        }
    }

    #[test]
    fn test_bootstrap_file_list_minimal() {
        let list = bootstrap_file_list(BootstrapMode::Minimal);
        assert_eq!(
            list,
            vec!["AGENTS.md", "SOUL.md", "IDENTITY.md", "USER.md", "TOOLS.md"]
        );
    }

    #[test]
    fn test_bootstrap_file_list_full() {
        let list = bootstrap_file_list(BootstrapMode::Full);
        assert_eq!(
            list,
            vec![
                "AGENTS.md",
                "SOUL.md",
                "IDENTITY.md",
                "USER.md",
                "TOOLS.md",
                "BOOTSTRAP.md",
                "MEMORY.md",
            ]
        );
    }

    #[test]
    fn test_load_bootstrap_files_invalid_workspace() {
        let result = load_bootstrap_files(Path::new("/nonexistent/path"), BootstrapMode::Full);
        assert!(result.is_err());
        match result.unwrap_err() {
            BootstrapLoaderError::InvalidWorkspace => {}
            _ => panic!("expected InvalidWorkspace"),
        }
    }

    #[test]
    fn test_load_bootstrap_files_partial_existence() {
        let tmp = TempDir::new().unwrap();
        create_test_files(tmp.path(), &["AGENTS.md", "SOUL.md"]);

        let result = load_bootstrap_files(tmp.path(), BootstrapMode::Minimal).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key("AGENTS.md"));
        assert!(result.contains_key("SOUL.md"));
        assert!(!result.contains_key("IDENTITY.md"));
    }

    #[test]
    fn test_load_bootstrap_files_full_mode() {
        let tmp = TempDir::new().unwrap();
        create_test_files(tmp.path(), &["AGENTS.md", "BOOTSTRAP.md", "MEMORY.md"]);

        let result = load_bootstrap_files(tmp.path(), BootstrapMode::Full).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.contains_key("AGENTS.md"));
        assert!(result.contains_key("BOOTSTRAP.md"));
        assert!(result.contains_key("MEMORY.md"));
    }

    #[test]
    fn test_heartbeat_not_in_any_mode() {
        let minimal = bootstrap_file_list(BootstrapMode::Minimal);
        let full = bootstrap_file_list(BootstrapMode::Full);
        assert!(!minimal.contains(&"HEARTBEAT.md"));
        assert!(!full.contains(&"HEARTBEAT.md"));
    }
}
