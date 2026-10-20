//! Contract tests for [`super::check_script`] — the shared driver for the
//! script-driven e2e check suites. Only the helper's own behavior is locked
//! here (script path resolution, fixture identity injection, result
//! plumbing, assertion helpers); suite-level script behavior stays in the
//! two consuming suites.

use std::path::PathBuf;
use std::process::Command;

use super::check_script::{
    assert_exit, assert_passed_marker, git, CheckResult, CheckScript, TempRepo,
};

/// Builds a real [`CheckResult`] from a trivial `bash -c` snippet via the
/// same [`CheckResult::from_output`] path the driver uses — no suite
/// script involved, so nothing here duplicates suite behavior tests.
fn check_result_from_bash(snippet: &str) -> CheckResult {
    let out = Command::new("bash")
        .args(["-c", snippet])
        .output()
        .expect("spawn bash -c");
    CheckResult::from_output(out)
}

/// Normal path: `CheckScript::new` resolves the script to an absolute path
/// under `<CARGO_MANIFEST_DIR>/scripts/`. Probed indirectly with a script
/// name that does not exist: bash fails to open it and echoes the fully
/// resolved path into stderr.
#[test]
fn test_check_script_new_resolves_script_under_manifest_scripts_dir() {
    let dir = tempfile::TempDir::new().expect("create temp dir under system temp dir");
    let missing = "check_script_probe_missing.sh";
    let expected = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(missing);

    let res = CheckScript::new(missing, "identity").run(dir.path(), &[]);

    assert_eq!(
        res.code(),
        Some(127),
        "bash exits 127 when the script file is missing"
    );
    let stderr = res.stderr.clone();
    assert!(
        stderr.contains(&expected.display().to_string()),
        "expected stderr to mention resolved path {}, got: {stderr}",
        expected.display()
    );
}

/// Normal path: `TempRepo::new(identity)` derives the fixture commit
/// author from the injected identity — `user.name` is the identity string
/// and `user.email` is `<identity>@example.invalid` — and `commit_all`
/// stages and commits the working tree (nested parents auto-created by
/// `write`).
#[test]
fn test_temp_repo_commit_all_uses_injected_identity() {
    let repo = TempRepo::new("check-script-fixture-author");
    assert!(repo.path().is_dir(), "temp repo dir must exist");

    repo.write("nested/dir/fixture.txt", "content\n");
    repo.stage("nested/dir/fixture.txt");
    repo.commit_all();

    let name = String::from_utf8(git(repo.path(), &["log", "-1", "--format=%an"]).stdout)
        .expect("author name is UTF-8");
    let email = String::from_utf8(git(repo.path(), &["log", "-1", "--format=%ae"]).stdout)
        .expect("author email is UTF-8");
    assert_eq!(name, "check-script-fixture-author\n");
    assert_eq!(email, "check-script-fixture-author@example.invalid\n");
}

/// Contract: [`CheckResult::output`] is stdout concatenated with stderr,
/// in that order.
#[test]
fn test_check_result_output_is_stdout_then_stderr() {
    let res = check_result_from_bash("echo to-out; echo to-err >&2");
    assert_eq!(res.output(), "to-out\nto-err\n");
}

/// Contract: [`CheckResult::code`] surfaces the child exit code.
#[test]
fn test_check_result_code_returns_exit_code() {
    let res = check_result_from_bash("exit 3");
    assert_eq!(res.code(), Some(3));
}

/// Hit: `assert_exit` passes when the actual code matches the expectation.
#[test]
fn test_assert_exit_passes_when_code_matches() {
    let res = check_result_from_bash("exit 2");
    assert_exit(&res, 2, "unit-ctx");
}

/// Miss: `assert_exit` panics with context, expected and actual code.
#[test]
#[should_panic(expected = "unit-ctx: expect exit 0, got Some(2)")]
fn test_assert_exit_panics_with_expected_and_actual_code() {
    let res = check_result_from_bash("echo boom; exit 2");
    assert_exit(&res, 0, "unit-ctx");
}

/// Hit: `assert_passed_marker` passes when the marker appears in stdout.
#[test]
fn test_assert_passed_marker_passes_when_marker_in_stdout() {
    let res = check_result_from_bash("echo 'env var check passed'");
    assert_passed_marker(&res, "env var check passed", "unit-ctx");
}

/// Miss: the marker is matched against stdout only — a marker echoed to
/// stderr must panic, and the message must carry context plus the full
/// merged output for diagnosis.
#[test]
#[should_panic(expected = "unit-ctx: expect passed marker in stdout")]
fn test_assert_passed_marker_panics_when_marker_missing_from_stdout() {
    let res = check_result_from_bash("echo 'env var check passed' >&2");
    assert_passed_marker(&res, "env var check passed", "unit-ctx");
}
