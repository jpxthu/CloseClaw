//! Shared driver for the script-driven e2e check suites
//! (`scripts/check-line-width.sh`, `scripts/check-env-var.sh`).
//!
//! Single implementation of the temp-git-repo + script-run infrastructure
//! previously duplicated near line-for-line in `line_width_check_tests`
//! and `env_var_check_tests` — STANDARDS §10: shared helper extracted
//! into a common module instead of re-implemented per test file.
//!
//! Parameterization points (the only per-suite differences):
//! - script path: [`CheckScript::new`] joins `CARGO_MANIFEST_DIR` with
//!   `scripts/<script_name>`;
//! - git identity: one identity string derives the fixture-commit author
//!   (`user.name=<identity>` / `user.email=<identity>@example.invalid`);
//! - passed marker: [`assert_passed_marker`] takes the suite's marker
//!   literal ("line width check passed" / "env var check passed").
//!
//! Responsibility split: [`TempRepo`] only manages the throwaway git
//! repository (write / stage / commit); [`CheckScript`] only drives the
//! check script against a directory.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

use closeclaw_common::test_helpers::ensure_dir;

/// Parameterization anchor: check-script path + git identity string. Each
/// suite constructs its own instance once; test functions drive the script
/// through [`CheckScript::run`] / [`CheckScript::run_with_env`] and build
/// identity-injected fixture repos through [`CheckScript::temp_repo`].
pub struct CheckScript {
    /// Absolute path of the check script: `CARGO_MANIFEST_DIR` +
    /// `scripts/<script_name>`.
    script: PathBuf,
    /// git identity base string; derives the fixture commit's `user.name`
    /// and `user.email`.
    identity: String,
}

impl CheckScript {
    /// Builds from `scripts/<script_name>` (relative to this repo's
    /// manifest root) and the git identity base string.
    pub fn new(script_name: &str, identity: &str) -> Self {
        Self {
            script: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("scripts")
                .join(script_name),
            identity: identity.to_string(),
        }
    }

    /// Resolved absolute path of the check script.
    pub fn script_path(&self) -> &Path {
        &self.script
    }

    /// Builds a fixture repo injected with this instance's git identity.
    pub fn temp_repo(&self) -> TempRepo {
        TempRepo::new(&self.identity)
    }

    /// Runs the check script in `dir` with arbitrary args (`dir` as CWD).
    pub fn run(&self, dir: &Path, args: &[&str]) -> CheckResult {
        self.run_with_env(dir, args, |_| {})
    }

    /// Variant: allows extra env adjustments on the child `Command` before
    /// spawn (e.g. `GIT_CEILING_DIRECTORIES` injection). Otherwise identical
    /// to [`CheckScript::run`]; the adjustment applies to this invocation
    /// only.
    pub fn run_with_env<F>(&self, dir: &Path, args: &[&str], adjust_env: F) -> CheckResult
    where
        F: FnOnce(&mut Command),
    {
        let mut cmd = Command::new("bash");
        cmd.arg(&self.script).args(args).current_dir(dir);
        adjust_env(&mut cmd);
        let out: Output = cmd
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn bash {}: {e}", self.script.display()));
        CheckResult::from_output(out)
    }
}

/// Isolated throwaway git repository: `TempDir` under the system temp dir
/// (follows `$TMPDIR`), cleaned up automatically on `Drop`.
pub struct TempRepo {
    dir: tempfile::TempDir,
    /// git identity base string; `commit_all` derives the fixture commit
    /// author's `user.name` (`= identity`) and `user.email`
    /// (`<identity>@example.invalid`).
    identity: String,
}

impl TempRepo {
    /// `git init`s an empty repo (the default-branch warning is harmless,
    /// output captured); `identity` derives the fixture commit's
    /// user.name / user.email.
    pub fn new(identity: &str) -> Self {
        let dir = tempfile::TempDir::new().expect("create temp dir under system temp dir");
        git(dir.path(), &["init", "-q"]);
        Self {
            dir,
            identity: identity.to_string(),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Writes a fixture file (parent dirs auto-created).
    pub fn write(&self, rel: &str, content: &str) {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            ensure_dir(parent).expect("create fixture parent dir");
        }
        std::fs::write(&path, content).expect("write fixture file");
    }

    /// Stages the given file (data source for the incremental mode: staged
    /// diff).
    pub fn stage(&self, rel: &str) {
        git(self.path(), &["add", rel]);
    }

    /// Stages everything and commits the working tree (injects the user
    /// identity; disables signing and global hooks for isolation).
    pub fn commit_all(&self) {
        git(self.path(), &["add", "-A", "-f"]);
        git(
            self.path(),
            &[
                "-c",
                &format!("user.name={}", self.identity),
                "-c",
                &format!("user.email={}@example.invalid", self.identity),
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-q",
                "-m",
                "test fixture",
            ],
        );
    }
}

/// Result of one check-script run.
pub struct CheckResult {
    status: ExitStatus,
    /// Script stdout (UTF-8 lossy).
    pub stdout: String,
    /// Script stderr (UTF-8 lossy).
    pub stderr: String,
}

impl CheckResult {
    /// Builds from raw process output (stdout/stderr decoded UTF-8 lossy).
    pub fn from_output(out: Output) -> Self {
        Self {
            status: out.status,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// Merged stdout + stderr (messages emitted via `echo` land on stdout,
    /// so asserting over the merge is more robust).
    pub fn output(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }
}

/// Runs a git subcommand in `dir`; on failure the assertion message
/// includes stdout/stderr.
pub fn git(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to spawn git");
    assert!(
        out.status.success(),
        "git {:?} failed (exit {:?})\nstdout: {}\nstderr: {}",
        args,
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Asserts the exit code; the failure message carries the full output for
/// diagnosis.
pub fn assert_exit(res: &CheckResult, expected: i32, ctx: &str) {
    assert_eq!(
        res.code(),
        Some(expected),
        "{ctx}: expect exit {expected}, got {:?}\noutput:\n{}",
        res.code(),
        res.output()
    );
}

/// Asserts the pass path's stdout carries the suite's passed marker
/// (parameterized marker literal).
pub fn assert_passed_marker(res: &CheckResult, marker: &str, ctx: &str) {
    assert!(
        res.stdout.contains(marker),
        "{ctx}: expect passed marker in stdout, got:\n{}",
        res.output()
    );
}
