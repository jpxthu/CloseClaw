//! 行为测试：`scripts/check-line-width.sh`（pre-commit 行宽硬限检查脚本）。
//!
//! 只驱动脚本的 `staged` 模式（含无参默认形态），不拉起完整 pre-commit hook
//! （hook 的文件行数上限、`cargo fmt`、角色规则不在本测试范围）。每个用例在
//! 独立的临时 git 仓库（`tempfile::TempDir`，落系统临时目录（跟随 $TMPDIR）、
//! Drop 自动清理）中执行，用例间无共享状态；全程无网络、无真实 LLM。
//!
//! 归档依据（docs/developer/STANDARDS.md）：§1 spawn 独立脚本进程 → e2e 档；
//! §2/§3 `tests/e2e/` 单 binary + 复数 `_tests.rs` 命名；§8 临时文件走 TempDir。
//!
//! 口径提示：行宽按字符（Unicode code point）计、非字节；.rs 只查注释行
//! （`//`、`///`、`//!`，允许前导空白），Cargo.toml 查全部新增行；只判定
//! staged 新增（+）行，存量超宽行不在增量口径内。

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

use closeclaw_common::test_helpers::ensure_dir;

/// 行宽上限：与脚本实现和 CONTRIBUTING.md「硬性限制」保持一致。
const MAX_WIDTH: usize = 100;

/// 夹具：以 `prefix` 起始、总宽恰为 `width` 字符的注释行（`x` 填充至目标宽度）。
fn comment_line(prefix: &str, width: usize) -> String {
    assert!(
        width > prefix.chars().count(),
        "fixture width must exceed prefix length"
    );
    format!("{prefix}{}", "x".repeat(width - prefix.chars().count()))
}

/// 夹具：总宽恰为 `width` 字符的 Cargo.toml 依赖行（toml 无 fmt 管辖，全行计数）。
fn toml_line(width: usize) -> String {
    let prefix = "key = \"";
    assert!(width > prefix.len() + 1, "fixture width too small");
    format!("{prefix}{}\"", "v".repeat(width - prefix.len() - 1))
}

/// 夹具：总宽恰为 `width` 字符的普通代码行（非注释，行宽归 cargo fmt 管辖）。
fn code_line(width: usize) -> String {
    let prefix = "pub const S: &str = \"";
    assert!(width > prefix.len() + 2, "fixture width too small");
    format!("{prefix}{}\";", "a".repeat(width - prefix.len() - 2))
}

/// 夹具：`// ` + `n_chars` 个中文字符的注释行（总宽 3 + n_chars 字符，1 字符 3 字节）。
fn chinese_comment(n_chars: usize) -> String {
    format!("// {}", "中".repeat(n_chars))
}

/// 隔离的临时 git 仓库：`TempDir` 落系统临时目录（跟随 $TMPDIR），Drop 时自动清理。
struct TempRepo {
    dir: tempfile::TempDir,
}

impl TempRepo {
    /// `git init` 一个空仓库（默认分支 warning 无害，输出被捕获）。
    fn new() -> Self {
        let dir = tempfile::TempDir::new().expect("create temp dir under system temp dir");
        git(dir.path(), &["init", "-q"]);
        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// 写入夹具文件（自动创建父目录）。
    fn write(&self, rel: &str, content: &str) {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            ensure_dir(parent).expect("create fixture parent dir");
        }
        std::fs::write(&path, content).expect("write fixture file");
    }

    /// 暂存指定文件（增量口径的数据源：staged diff）。
    fn stage(&self, rel: &str) {
        git(self.path(), &["add", rel]);
    }

    /// 全量暂存并提交当前工作树（内联 user 身份，关掉签名与全局 hooks 保证隔离）。
    fn commit_all(&self) {
        git(self.path(), &["add", "-A", "-f"]);
        git(
            self.path(),
            &[
                "-c",
                "user.name=line-width-check-tests",
                "-c",
                "user.email=line-width-check-tests@example.invalid",
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

    /// 以本仓库为 CWD 运行检查脚本 staged 模式。
    fn check(&self) -> CheckResult {
        run_script(self.path(), &["staged"])
    }
}

/// 在 `dir` 中执行 git 子命令；失败时附带 stdout/stderr 断言输出。
fn git(dir: &Path, args: &[&str]) -> Output {
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

/// 检查脚本的一次运行结果。
struct CheckResult {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

impl CheckResult {
    /// stdout + stderr 合并（文案经 `echo` 走 stdout，合并断言更稳）。
    fn output(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    fn code(&self) -> Option<i32> {
        self.status.code()
    }
}

/// 在 `dir` 中以任意参数运行检查脚本（脚本取本仓库真实路径，`dir` 为 CWD）。
fn run_script(dir: &Path, args: &[&str]) -> CheckResult {
    run_script_with_env(dir, args, |_| {})
}

/// 变体：启动前允许对子进程 `Command` 做额外 env 调整（如 `GIT_CEILING_DIRECTORIES`
/// 注入），其余行为与 [`run_script`] 完全一致；不影响其它用例。
fn run_script_with_env<F>(dir: &Path, args: &[&str], adjust_env: F) -> CheckResult
where
    F: FnOnce(&mut Command),
{
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/check-line-width.sh");
    let mut cmd = Command::new("bash");
    cmd.arg(script).args(args).current_dir(dir);
    adjust_env(&mut cmd);
    let out: Output = cmd
        .output()
        .expect("failed to spawn bash scripts/check-line-width.sh");
    CheckResult {
        status: out.status,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// 断言退出码，失败信息附完整输出便于定位。
fn assert_exit(res: &CheckResult, expected: i32, ctx: &str) {
    assert_eq!(
        res.code(),
        Some(expected),
        "{ctx}: expect exit {expected}, got {:?}\noutput:\n{}",
        res.code(),
        res.output()
    );
}

/// 断言通过路径的 stdout 携带 passed 标记。
fn assert_passed_marker(res: &CheckResult, ctx: &str) {
    assert!(
        res.stdout.contains("line width check passed"),
        "{ctx}: expect passed marker in stdout, got:\n{}",
        res.output()
    );
}

/// 正常路径：`git init` 后无任何 staged 目标文件（空 index）→ exit 0 并输出 passed。
/// 同时钉住无参调用形态（默认模式 = staged）。
#[test]
fn test_line_width_empty_index_passes() {
    let repo = TempRepo::new();

    let res = run_script(repo.path(), &[]);
    assert_exit(&res, 0, "empty staged index must pass via default mode");
    assert_passed_marker(&res, "empty staged index");
}

/// 正常路径 + 边界：staged 的非目标文件（.md，含 150 字符超宽行）不在扫描范围。
#[test]
fn test_line_width_non_target_file_passes() {
    let repo = TempRepo::new();
    let long = "m".repeat(150);
    repo.write("notes.md", &format!("# notes\n{long}\n"));
    repo.stage("notes.md");

    let res = repo.check();
    assert_exit(&res, 0, "staged non-target file must be out of scan scope");
    assert_passed_marker(&res, "non-target staged file");
}

/// 正常路径 + 边界：恰 100 字符（上限值）的注释行与 Cargo.toml 行均放行。
#[test]
fn test_line_width_exactly_100_passes() {
    let repo = TempRepo::new();
    repo.write(
        "src/edge.rs",
        &format!("pub fn f() {{}}\n{}\n", comment_line("// ", MAX_WIDTH)),
    );
    repo.write(
        "Cargo.toml",
        &format!("[package]\n{}\n", toml_line(MAX_WIDTH)),
    );
    repo.stage("src/edge.rs");
    repo.stage("Cargo.toml");

    let res = repo.check();
    assert_exit(&res, 0, "exactly 100-char lines must pass");
    assert_passed_marker(&res, "boundary 100-char lines");
}

/// 错误路径：101 字符 `//` 注释行拦截，文案含 文件:行号:长度 与修复指引。
#[test]
fn test_line_width_flags_101_slash_comment() {
    let repo = TempRepo::new();
    let line = comment_line("// ", MAX_WIDTH + 1);
    repo.write("src/bad.rs", &format!("pub fn f() {{}}\n{line}\n"));
    repo.stage("src/bad.rs");

    let res = repo.check();
    assert_exit(&res, 1, "101-char // comment must be flagged");
    let out = res.output();
    assert!(
        out.contains(&format!("src/bad.rs:2:{}", MAX_WIDTH + 1)),
        "expect location src/bad.rs:2:101, got:\n{out}"
    );
    assert!(
        out.contains("修复指引"),
        "expect fix guidance in message, got:\n{out}"
    );
}

/// 错误路径：`///` 与 `//!` 起始的注释行同样按注释口径拦截，行号逐一对应。
#[test]
fn test_line_width_flags_doc_comments() {
    let repo = TempRepo::new();
    let doc = comment_line("/// ", MAX_WIDTH + 1);
    let inner = comment_line("//! ", MAX_WIDTH + 2);
    repo.write("src/docs.rs", &format!("{doc}\n{inner}\n"));
    repo.stage("src/docs.rs");

    let res = repo.check();
    assert_exit(&res, 1, "/// and //! overwidth comments must be flagged");
    let out = res.output();
    assert!(
        out.contains(&format!("src/docs.rs:1:{}", MAX_WIDTH + 1)),
        "expect doc comment at src/docs.rs:1, got:\n{out}"
    );
    assert!(
        out.contains(&format!("src/docs.rs:2:{}", MAX_WIDTH + 2)),
        "expect inner doc comment at src/docs.rs:2, got:\n{out}"
    );
}

/// 错误路径 + basename 覆盖：嵌套路径 crates/app/Cargo.toml 的新增行同样拦截。
#[test]
fn test_line_width_flags_nested_cargo_toml() {
    let repo = TempRepo::new();
    repo.write(
        "crates/app/Cargo.toml",
        &format!("[dependencies]\n{}\n", toml_line(MAX_WIDTH + 1)),
    );
    repo.stage("crates/app/Cargo.toml");

    let res = repo.check();
    assert_exit(&res, 1, "nested Cargo.toml overwidth line must be flagged");
    let out = res.output();
    assert!(
        out.contains(&format!("crates/app/Cargo.toml:2:{}", MAX_WIDTH + 1)),
        "expect location crates/app/Cargo.toml:2:101, got:\n{out}"
    );
}

/// 边界：前导空白 + 超宽注释行拦截（前导空白不破坏注释识别，宽度按原行计）。
#[test]
fn test_line_width_flags_indented_comment() {
    let repo = TempRepo::new();
    // 4 空格缩进 + 97 字符注释 = 101 字符
    let line = format!("    {}", comment_line("// ", MAX_WIDTH - 3));
    assert_eq!(line.chars().count(), MAX_WIDTH + 1, "fixture layout");
    repo.write("src/indent.rs", &format!("pub fn f() {{}}\n{line}\n"));
    repo.stage("src/indent.rs");

    let res = repo.check();
    assert_exit(&res, 1, "indented overwidth comment must be flagged");
    let out = res.output();
    assert!(
        out.contains(&format!("src/indent.rs:2:{}", MAX_WIDTH + 1)),
        "expect location src/indent.rs:2:101, got:\n{out}"
    );
}

/// 边界：非注释的超宽 .rs 代码行放行（代码行格式与行宽归 cargo fmt 管辖，不重复查）。
#[test]
fn test_line_width_non_comment_code_passes() {
    let repo = TempRepo::new();
    repo.write(
        "src/long_code.rs",
        &format!("{}\n", code_line(MAX_WIDTH + 20)),
    );
    repo.stage("src/long_code.rs");

    let res = repo.check();
    assert_exit(&res, 0, "overwidth non-comment code line is fmt's business");
    assert_passed_marker(&res, "non-comment overwidth code line");
}

/// 增量口径：存量（HEAD 既有）超宽注释行不在 staged 新增行内 → 放行。
#[test]
fn test_line_width_legacy_line_passes() {
    let repo = TempRepo::new();
    let legacy = comment_line("// ", MAX_WIDTH + 1);
    repo.write("src/legacy.rs", &format!("{legacy}\npub fn old() {{}}\n"));
    repo.commit_all();

    // 只在文件末尾追加干净行并 stage：存量超宽行在 diff 中为上下文行，不判定
    repo.write(
        "src/legacy.rs",
        &format!("{legacy}\npub fn old() {{}}\npub fn new() {{}}\n"),
    );
    repo.stage("src/legacy.rs");

    let res = repo.check();
    assert_exit(&res, 0, "legacy (context) overwidth line must pass");
    assert_passed_marker(&res, "legacy overwidth line untouched");
}

/// 边界：中文注释行按字符计——40 中文字符（123 字节）通过；>100 字符中文行拦截。
#[test]
fn test_line_width_chinese_counted_by_chars() {
    let repo = TempRepo::new();

    // 43 字符 / 123 字节：字节口径会误拦（123 > 100），字符口径必须放行
    let ok = chinese_comment(40);
    assert_eq!(ok.chars().count(), 43, "fixture: 3 prefix + 40 chars");
    assert_eq!(ok.len(), 123, "fixture: 3 ascii bytes + 40 * 3 bytes");
    repo.write("src/cn_ok.rs", &format!("pub fn f() {{}}\n{ok}\n"));
    repo.stage("src/cn_ok.rs");
    let res = repo.check();
    assert_exit(&res, 0, "40-Chinese-char comment must pass by char count");
    assert_passed_marker(&res, "chinese comment within 100 chars");

    // 103 字符：超上限 → 拦截（字符口径，非字节放大）
    let over = chinese_comment(MAX_WIDTH);
    assert_eq!(over.chars().count(), 103, "fixture: 3 prefix + 100 chars");
    repo.write("src/cn_over.rs", &format!("pub fn f() {{}}\n{over}\n"));
    repo.stage("src/cn_over.rs");
    let res = repo.check();
    assert_exit(&res, 1, "over-100-Chinese-char comment must be flagged");
    let out = res.output();
    assert!(
        out.contains(&format!("src/cn_over.rs:2:{}", MAX_WIDTH + 3)),
        "expect location src/cn_over.rs:2:103, got:\n{out}"
    );
}

/// 状态转换：同一文件先 stage 超宽行被拦，修复后重 stage 由拦转放。
#[test]
fn test_line_width_restage_fix_flips_to_pass() {
    let repo = TempRepo::new();
    let bad = comment_line("// ", MAX_WIDTH + 1);
    repo.write("src/fix.rs", &format!("pub fn f() {{}}\n{bad}\n"));
    repo.stage("src/fix.rs");
    let res = repo.check();
    assert_exit(&res, 1, "overwidth staged comment must be flagged first");

    let fixed = comment_line("// ", MAX_WIDTH);
    repo.write("src/fix.rs", &format!("pub fn f() {{}}\n{fixed}\n"));
    repo.stage("src/fix.rs");
    let res = repo.check();
    assert_exit(&res, 0, "restaged fixed file must pass");
    assert_passed_marker(&res, "restaged fixed file");
}

/// 边界 + 行号口径：多 hunk 场景下第二 hunk 中超宽注释行的报出行号正确
/// （逐 @@ 独立记账，不跨 hunk 累加）。
///
/// 夹具：基线 15 行短代码行提交后，在 f02 之后插入超宽注释 A（新文件第 3 行）、
/// 在 f13 之后插入超宽注释 B（新文件第 15 行）。两处改动间隔 > 2×默认上下文，
/// diff 拆为两个 hunk；若行号跨 hunk 累加，第二 hunk 的报出行号必然错位。
#[test]
fn test_line_width_multi_hunk_second_hunk_line_number() {
    let repo = TempRepo::new();
    let base: Vec<String> = (1..=15).map(|i| format!("pub fn f{i:02}() {{}}")).collect();
    repo.write("src/multi.rs", &format!("{}\n", base.join("\n")));
    repo.commit_all();

    let bad = comment_line("// ", MAX_WIDTH + 1);
    let mut edited = base.clone();
    edited.insert(2, bad.clone()); // 新文件第 3 行
    edited.insert(14, bad); // 新文件第 15 行（f13 之后）
    assert_eq!(edited[2].chars().count(), MAX_WIDTH + 1, "fixture hunk 1");
    assert_eq!(edited[14].chars().count(), MAX_WIDTH + 1, "fixture hunk 2");
    repo.write("src/multi.rs", &format!("{}\n", edited.join("\n")));
    repo.stage("src/multi.rs");

    let res = repo.check();
    assert_exit(&res, 1, "both hunks' overwidth comments must be flagged");
    let out = res.output();
    assert!(
        out.contains(&format!("src/multi.rs:3:{}", MAX_WIDTH + 1)),
        "expect hunk 1 hit at src/multi.rs:3:101, got:\n{out}"
    );
    assert!(
        out.contains(&format!("src/multi.rs:15:{}", MAX_WIDTH + 1)),
        "expect hunk 2 hit at src/multi.rs:15:101 (per-@@ accounting), got:\n{out}"
    );
}

/// 契约：非法参数（未知模式 / 参数个数 > 1）→ exit 2，用法提示走 stderr。
#[test]
fn test_line_width_invalid_args_exit_2() {
    let repo = TempRepo::new();

    let unknown = run_script(repo.path(), &["bogus"]);
    assert_exit(&unknown, 2, "unknown mode must exit 2");
    assert!(
        unknown.stderr.contains("usage:"),
        "usage hint must go to stderr, got:\n{}",
        unknown.output()
    );

    let too_many = run_script(repo.path(), &["staged", "staged"]);
    assert_exit(&too_many, 2, "more than one argument must exit 2");
    assert!(
        too_many.stderr.contains("usage:"),
        "usage hint must go to stderr, got:\n{}",
        too_many.output()
    );
}

/// 契约：非 git 目录（仓库守卫）→ exit 2，且报出守卫文案而非静默通过。
///
/// 夹具目录可能落在 git 工作树内（如 TMPDIR 指向仓库内路径）：git 向上搜索会
/// 发现仓库根的 .git，「非 git 目录」前提失效。注入 `GIT_CEILING_DIRECTORIES`
/// 指向夹具父目录，使 git 的向上搜索在该处停止（TMPDIR 位于 git 工作树内/外
/// 行为一致）；同时清除可能继承的 GIT_DIR / GIT_WORK_TREE，排除子进程 env 干扰。
#[test]
fn test_line_width_non_git_dir_exits_2() {
    let dir = tempfile::TempDir::new().expect("create temp dir under system temp dir");

    let res = run_script_with_env(dir.path(), &["staged"], |cmd| {
        // git 向上搜索按物理路径比对，且 ceiling 不解析 symlink，故注入
        // canonicalize 后的父目录；canonicalize 失败时回退字面父目录。
        let parent = dir.path().parent().unwrap_or_else(|| Path::new("/"));
        let ceiling = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        cmd.env("GIT_CEILING_DIRECTORIES", ceiling);
        cmd.env_remove("GIT_DIR");
        cmd.env_remove("GIT_WORK_TREE");
    });
    assert_exit(&res, 2, "non-git directory must exit 2");
    let out = res.output();
    assert!(
        out.contains("必须在 git 仓库内运行"),
        "expect non-git guard message, got:\n{out}"
    );
}
