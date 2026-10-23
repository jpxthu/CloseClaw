//! Mock clawhub CLI and failure-propagation assertion helpers for discovery tests.
//!
//! # 硬性约定（本模块所有测试适用）
//!
//! - 创建并实际 spawn mock 脚本的用例必须加 `#[serial_test::serial]`
//!   （ETXTBSY 根因）。
//! - mock 脚本必须落 `tempfile::TempDir`（系统临时目录），禁止仓库固定路径。

use super::SkillDiscoverySkill;
use crate::registry::{Skill, SkillError};
use tempfile::TempDir;

/// Fixed stdout emitted by [`create_mock_clawhub`] for the `search`
/// subcommand (exactly what `results` carries after the skill trims it).
/// Must not contain single quotes — the generated script embeds it in a
/// single-quoted shell string.
pub(super) const MOCK_SEARCH_OUTPUT: &str = "skill-alpha 1.0.0 (verified)\nskill-beta 0.3.1";

/// Fixed stdout emitted by [`create_mock_clawhub`] for the `list`
/// subcommand (exactly what `installed` carries after the skill trims it).
/// Must not contain single quotes — same constraint as above.
pub(super) const MOCK_LIST_OUTPUT: &str = "skill-alpha 1.0.0\nskill-gamma 2.4.0";

/// Fixed stderr message emitted by [`create_failing_mock_clawhub`].
/// Must not contain single quotes — same constraint as above.
pub(super) const MOCK_FAILURE_MESSAGE: &str = "mock clawhub: simulated failure";

/// Write a mock shell script atomically — temp-name write, handle
/// closed, rename into place, chmod 0755 — so the exec path never coexists
/// with an open write handle (avoids ETXTBSY on spawn).
pub(super) fn write_mock_script(path: &std::path::Path, content: &str) {
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let script_tmp = std::path::PathBuf::from(tmp_name);
    std::fs::write(&script_tmp, content).unwrap(); // write & close before rename
    std::fs::rename(&script_tmp, path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, PermissionsExt::from_mode(0o755)).unwrap();
    }
}

/// Create a mock clawhub script under `tmp`:
/// - `search <query>` → [`MOCK_SEARCH_OUTPUT`] on stdout, exit 0
/// - `list` → [`MOCK_LIST_OUTPUT`] on stdout, exit 0
/// - any other invocation → message on stderr, exit 1
///
/// Returns the script path (pass to
/// `SkillDiscoverySkill::with_cli_command`).
pub(super) fn create_mock_clawhub(tmp: &TempDir) -> String {
    let script_path = tmp.path().join("mock_clawhub.sh");
    let content = format!(
        r#"#!/bin/bash
case "$1" in
  search)
    echo '{MOCK_SEARCH_OUTPUT}'
    exit 0
    ;;
  list)
    echo '{MOCK_LIST_OUTPUT}'
    exit 0
    ;;
  *)
    echo "mock clawhub: unknown subcommand: $1" >&2
    exit 1
    ;;
esac
"#
    );
    write_mock_script(&script_path, &content);
    script_path.to_str().unwrap().to_string()
}

/// Create a mock clawhub script under `tmp` that always fails: emits
/// [`MOCK_FAILURE_MESSAGE`] to stderr and exits 1. Returns the script path
/// (pass to `SkillDiscoverySkill::with_cli_command`).
pub(super) fn create_failing_mock_clawhub(tmp: &TempDir) -> String {
    let script_path = tmp.path().join("failing_mock_clawhub.sh");
    let content = format!("#!/bin/bash\necho '{MOCK_FAILURE_MESSAGE}' >&2\nexit 1\n");
    write_mock_script(&script_path, &content);
    script_path.to_str().unwrap().to_string()
}

/// Assert that executing the discovery skill with `payload` against a
/// failing mock clawhub ([`create_failing_mock_clawhub`]) propagates the
/// failure: `execute` must return `SkillError::ExecutionFailed` whose
/// message contains [`MOCK_FAILURE_MESSAGE`].
///
/// - `payload` is the full `execute` argument (must carry the `action`
///   to exercise).
/// - `action` only appears in the panic message for diagnostics.
/// - Spawns a mock script, so the calling test must be marked
///   `#[serial_test::serial]` (see the module-level 硬性约定).
pub(super) async fn assert_clawhub_failure(payload: serde_json::Value, action: &str) {
    let tmp = TempDir::new().unwrap();
    let mock = create_failing_mock_clawhub(&tmp);
    let skill = SkillDiscoverySkill::with_cli_command(mock);
    let err = skill.execute(Some(payload)).await.unwrap_err();
    let SkillError::ExecutionFailed(msg) = err else {
        panic!("clawhub {action} failure should return ExecutionFailed, got: {err:?}");
    };
    assert!(
        msg.contains(MOCK_FAILURE_MESSAGE),
        "expected stderr containing {MOCK_FAILURE_MESSAGE:?}, got: {msg}"
    );
}
