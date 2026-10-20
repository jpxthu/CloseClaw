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

// Transitional: consumed only once the two suites migrate to this module.
// Drop this allow as soon as both suites drive the shared helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

use closeclaw_common::test_helpers::ensure_dir;

/// 参数化锚点：检查脚本路径 + git 身份字符串。每个套件构造一次自己的实例，
/// 测试函数经 [`CheckScript::run`] / [`CheckScript::run_with_env`] 驱动脚本、
/// 经 [`CheckScript::temp_repo`] 构造注入了同身份的夹具仓库。
pub struct CheckScript {
    /// 检查脚本绝对路径：`CARGO_MANIFEST_DIR` + `scripts/<script_name>`。
    script: PathBuf,
    /// git 身份基串：派生 fixture 提交的 `user.name` 与 `user.email`。
    identity: String,
}

impl CheckScript {
    /// 以 `scripts/<script_name>`（相对本仓库 manifest 根）与 git 身份基串构造。
    pub fn new(script_name: &str, identity: &str) -> Self {
        Self {
            script: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("scripts")
                .join(script_name),
            identity: identity.to_string(),
        }
    }

    /// 构造注入了本实例 git 身份的夹具仓库。
    pub fn temp_repo(&self) -> TempRepo {
        TempRepo::new(&self.identity)
    }

    /// 在 `dir` 中以任意参数运行检查脚本（`dir` 为 CWD）。
    pub fn run(&self, dir: &Path, args: &[&str]) -> CheckResult {
        self.run_with_env(dir, args, |_| {})
    }

    /// 变体：启动前允许对子进程 `Command` 做额外 env 调整（如
    /// `GIT_CEILING_DIRECTORIES` 注入），其余行为与 [`CheckScript::run`]
    /// 完全一致；不影响其它用例。
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

/// 隔离的临时 git 仓库：`TempDir` 落系统临时目录（跟随 $TMPDIR），Drop 时自动清理。
pub struct TempRepo {
    dir: tempfile::TempDir,
    /// fixture 提交作者 `user.name`（= 身份基串）。
    user_name: String,
    /// fixture 提交作者 `user.email`（= `<身份基串>@example.invalid`）。
    user_email: String,
}

impl TempRepo {
    /// `git init` 一个空仓库（默认分支 warning 无害，输出被捕获）；
    /// `identity` 派生 fixture 提交的 user.name / user.email。
    pub fn new(identity: &str) -> Self {
        let dir = tempfile::TempDir::new().expect("create temp dir under system temp dir");
        git(dir.path(), &["init", "-q"]);
        Self {
            dir,
            user_name: identity.to_string(),
            user_email: format!("{identity}@example.invalid"),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// 写入夹具文件（自动创建父目录）。
    pub fn write(&self, rel: &str, content: &str) {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            ensure_dir(parent).expect("create fixture parent dir");
        }
        std::fs::write(&path, content).expect("write fixture file");
    }

    /// 暂存指定文件（增量口径的数据源：staged diff）。
    pub fn stage(&self, rel: &str) {
        git(self.path(), &["add", rel]);
    }

    /// 全量暂存并提交当前工作树（注入 user 身份，关掉签名与全局 hooks 保证隔离）。
    pub fn commit_all(&self) {
        git(self.path(), &["add", "-A", "-f"]);
        git(
            self.path(),
            &[
                "-c",
                &format!("user.name={}", self.user_name),
                "-c",
                &format!("user.email={}", self.user_email),
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

/// 检查脚本的一次运行结果。
pub struct CheckResult {
    status: ExitStatus,
    /// 脚本 stdout（UTF-8 lossy）。
    pub stdout: String,
    /// 脚本 stderr（UTF-8 lossy）。
    pub stderr: String,
}

impl CheckResult {
    /// 从原始进程输出构造（stdout/stderr 经 UTF-8 lossy 解码）。
    pub fn from_output(out: Output) -> Self {
        Self {
            status: out.status,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// stdout + stderr 合并（文案经 `echo` 走 stdout，合并断言更稳）。
    pub fn output(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }
}

/// 在 `dir` 中执行 git 子命令；失败时附带 stdout/stderr 断言输出。
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

/// 断言退出码，失败信息附完整输出便于定位。
pub fn assert_exit(res: &CheckResult, expected: i32, ctx: &str) {
    assert_eq!(
        res.code(),
        Some(expected),
        "{ctx}: expect exit {expected}, got {:?}\noutput:\n{}",
        res.code(),
        res.output()
    );
}

/// 断言通过路径的 stdout 携带本套件的 passed 标记（参数化标记串）。
pub fn assert_passed_marker(res: &CheckResult, marker: &str, ctx: &str) {
    assert!(
        res.stdout.contains(marker),
        "{ctx}: expect passed marker in stdout, got:\n{}",
        res.output()
    );
}
