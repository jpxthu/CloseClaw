//! 行为测试：`scripts/check-env-var.sh`（CI 与 pre-commit 共享的禁令检查脚本）。
//!
//! 只驱动脚本层的 `all` / `staged` 两种模式，不拉起完整 pre-commit hook
//! （hook 的行数上限、`cargo fmt`、角色规则不在本测试范围）。每个用例在独立
//! 的临时 git 仓库（`tempfile::TempDir`，落 /tmp、Drop 自动清理）中执行，
//! 用例间无共享状态；全程无网络、无真实 LLM。
//!
//! 归档依据（docs/developer/STANDARDS.md）：§1 spawn 独立脚本进程 → e2e 档；
//! §2/§3 `tests/e2e/` 单 binary + 复数 `_tests.rs` 命名；§8 临时文件走 TempDir。
//!
//! 源码书写约束：本文件会被 `check-env-var.sh all` 纳入扫描，任何一行都不得
//! 命中行级文本判定，因此禁令 token 一律经 [`BANNED_SET`] / [`BANNED_REMOVE`]
//! 片段常量拼装，源码中不得出现裸 token（否则测试文件自身会打红 CI 与 hook）。

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

/// 禁令 token 片段（`se` + `t_var`）：拆开声明，避免本测试源码自身被行级文本判定命中。
const BANNED_SET: &str = concat!("se", "t_var");
/// 禁令 token 片段（`re` + `move_var`）：同上，避免源码自身命中。
const BANNED_REMOVE: &str = concat!("re", "move_var");

/// 统一错误文案必须携带的正确引用：STANDARDS §7 章节。
const REF_STANDARDS: &str = "docs/developer/STANDARDS.md §7";
/// 统一错误文案必须携带的正确引用：唯一豁免点的真实路径。
const REF_EXEMPT_PATH: &str = "crates/daemon/src/mod.rs";
/// 统一错误文案必须携带的正确引用：CONTRIBUTING 安全红线章节。
const REF_CONTRIBUTING: &str = "CONTRIBUTING.md「测试 > 安全红线」";

/// 过期文案回归基线：历史 hook 引用了 CONTRIBUTING.md 中不存在的「环境变量禁令」章节。
const STALE_SECTION_REF: &str = "环境变量禁令";
/// 过期文案回归基线：顶层 `daemon/mod.rs` 不存在（真实路径为 crates/daemon/src/mod.rs）。
const STALE_EXEMPT_PATH: &str = "daemon/mod.rs";

/// 夹具：一行真实的禁令写入调用文本（等价真实调用形态）。
fn set_call(arg: &str) -> String {
    format!("std::env::{}(\"{}\", \"1\");", BANNED_SET, arg)
}

/// 夹具：一行真实的禁令删除调用文本。
fn remove_call(arg: &str) -> String {
    format!("std::env::{}(\"{}\");", BANNED_REMOVE, arg)
}

/// 夹具：行尾带 `load_env_file` 标记的豁免形态行（与唯一豁免点实例同形）。
fn exempt_call() -> String {
    format!(
        "std::env::{}(&key, &value); // load_env_file: allowed exception per CONTRIBUTING.md",
        BANNED_SET
    )
}

/// 夹具：注释/散文中提及禁令 token 的行（文本匹配口径下如实命中）。
fn comment_mention() -> String {
    format!("// prose mention of {} as documentation", BANNED_SET)
}

/// 隔离的临时 git 仓库：`TempDir` 落 /tmp，Drop 时自动清理。
struct TempRepo {
    dir: tempfile::TempDir,
}

impl TempRepo {
    /// `git init` 一个空仓库（默认分支 warning 无害，输出被捕获）。
    fn new() -> Self {
        let dir = tempfile::TempDir::new().expect("create temp dir under /tmp");
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
            std::fs::create_dir_all(parent).expect("create fixture parent dir");
        }
        std::fs::write(&path, content).expect("write fixture file");
    }

    /// 全量暂存并提交当前工作树（内联 user 身份，关掉签名与全局 hooks 保证隔离）。
    fn commit_all(&self) {
        git(self.path(), &["add", "-A", "-f"]);
        git(
            self.path(),
            &[
                "-c",
                "user.name=env-var-check-tests",
                "-c",
                "user.email=env-var-check-tests@example.invalid",
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

    /// 以本仓库为 CWD 运行检查脚本指定模式。
    fn check(&self, mode: &str) -> CheckResult {
        run_check(self.path(), mode)
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
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/check-env-var.sh");
    let out: Output = Command::new("bash")
        .arg(script)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to spawn bash scripts/check-env-var.sh");
    CheckResult {
        status: out.status,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// 运行 `bash scripts/check-env-var.sh <mode>`：脚本取本仓库真实路径，夹具仓库为 CWD。
fn run_check(dir: &Path, mode: &str) -> CheckResult {
    run_script(dir, &[mode])
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

/// 正常路径：干净的 tracked `.rs` 文件集 → exit 0 并输出 passed 标记。
#[test]
fn test_all_mode_clean_repo_passes() {
    let repo = TempRepo::new();
    repo.write(
        "src/lib.rs",
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    );
    repo.commit_all();

    let res = repo.check("all");
    assert_exit(&res, 0, "all mode on clean tracked .rs files");
    assert!(
        res.stdout.contains("env var check passed"),
        "expect passed marker in stdout, got:\n{}",
        res.output()
    );
}

/// 错误路径：写入与删除两类调用均命中 → exit 1、报出命中位置与正确引用。
#[test]
fn test_all_mode_reports_hit_locations_and_references() {
    let repo = TempRepo::new();
    let set_line = set_call("A");
    let remove_line = remove_call("B");
    repo.write(
        "src/bad.rs",
        &format!(
            "pub struct A;\n{}\npub fn f() {{}}\n{}\n",
            set_line, remove_line
        ),
    );
    repo.commit_all();

    let res = repo.check("all");
    assert_exit(&res, 1, "all mode with committed violations");
    let out = res.output();
    assert!(
        out.contains(&format!("src/bad.rs:2:{}", set_line)),
        "expect hit location src/bad.rs:2, got:\n{out}"
    );
    assert!(
        out.contains(&format!("src/bad.rs:4:{}", remove_line)),
        "expect hit location src/bad.rs:4, got:\n{out}"
    );
    assert!(
        out.contains(REF_STANDARDS),
        "expect reference {REF_STANDARDS}, got:\n{out}"
    );
    assert!(
        out.contains(REF_EXEMPT_PATH),
        "expect reference {REF_EXEMPT_PATH}, got:\n{out}"
    );
}

/// 豁免语义：行尾注释带 `load_env_file` 标记的调用行不命中 → exit 0。
#[test]
fn test_all_mode_load_env_file_marker_line_exempt() {
    let repo = TempRepo::new();
    repo.write(
        "src/env_loader.rs",
        &format!("pub fn load(p: &str) {{\n    {}\n}}\n", exempt_call()),
    );
    repo.commit_all();

    let res = repo.check("all");
    assert_exit(&res, 0, "line with load_env_file marker must be exempt");
}

/// 边界：同名标识符（前后缀粘连）不满足词边界，不得命中。
#[test]
fn test_all_mode_same_named_identifiers_not_flagged() {
    let repo = TempRepo::new();
    repo.write(
        "src/identifiers.rs",
        "\
let set_variable = 1;
let my_set_var = 2;
let set_var_ref = 3;
let remove_vars = 4;
",
    );
    repo.commit_all();

    let res = repo.check("all");
    assert_exit(
        &res,
        0,
        "word-boundary matching must skip same-named identifiers",
    );
}

/// 边界：注释散文中的 token 同样按行级文本如实命中（脚本口径，非调用才命中）。
#[test]
fn test_all_mode_comment_text_hits_like_call() {
    let repo = TempRepo::new();
    let mention = comment_mention();
    repo.write("src/notes.rs", &format!("{}\n", mention));
    repo.commit_all();

    let res = repo.check("all");
    assert_exit(&res, 1, "text matching hits comment mentions too");
    let out = res.output();
    assert!(
        out.contains(&format!("src/notes.rs:1:{}", mention)),
        "expect comment hit location src/notes.rs:1, got:\n{out}"
    );
}

/// staged 模式：staged 新增 `*.rs` 行中的违规 → exit 1 并报出位置与引用。
#[test]
fn test_staged_mode_flags_staged_new_rs_lines() {
    let repo = TempRepo::new();
    repo.write("src/base.rs", "pub fn base() {}\n");
    repo.commit_all();

    let staged_line = set_call("STAGED");
    repo.write(
        "src/new_viol.rs",
        &format!("pub fn g() {{}}\n{}\n", staged_line),
    );
    git(repo.path(), &["add", "src/new_viol.rs"]);

    let res = repo.check("staged");
    assert_exit(&res, 1, "staged new .rs line with violation must fail");
    let out = res.output();
    assert!(
        out.contains(&format!("src/new_viol.rs:2:{}", staged_line)),
        "expect staged hit location src/new_viol.rs:2, got:\n{out}"
    );
    assert!(
        out.contains(REF_STANDARDS),
        "expect reference {REF_STANDARDS}, got:\n{out}"
    );
}

/// staged 模式：工作树未 stage 的违规不判定（同时保底一个干净 staged 改动）。
#[test]
fn test_staged_mode_ignores_unstaged_violation() {
    let repo = TempRepo::new();
    repo.write("src/clean.rs", "pub fn clean() {}\n");
    repo.write("src/other.rs", "pub fn other() {}\n");
    repo.commit_all();

    // 违规写进工作树但不进 index
    repo.write(
        "src/clean.rs",
        &format!("pub fn clean() {{}}\n{}\n", remove_call("UNSTAGED")),
    );
    // 另一个干净的 staged 改动，确保脚本确实进入 staged 扫描分支
    repo.write("src/other.rs", "pub fn other() {}\npub fn other2() {}\n");
    git(repo.path(), &["add", "src/other.rs"]);

    let res = repo.check("staged");
    assert_exit(
        &res,
        0,
        "unstaged working-tree violation must not be flagged",
    );
}

/// staged 模式：HEAD/index 既有违规行（staged 改动在同文件其它行）不判定。
#[test]
fn test_staged_mode_ignores_untouched_head_violation() {
    let repo = TempRepo::new();
    let legacy_line = set_call("LEGACY");
    repo.write(
        "src/legacy.rs",
        &format!(
            "{}\npub fn before() {{}}\npub fn after() {{}}\n",
            legacy_line
        ),
    );
    repo.commit_all();

    // 只改第 2 行并 stage：既有违规行不是 staged 新增行（diff 中仅为上下文行），不判定
    repo.write(
        "src/legacy.rs",
        &format!(
            "{}\npub fn before_v2() {{}}\npub fn after() {{}}\n",
            legacy_line
        ),
    );
    git(repo.path(), &["add", "src/legacy.rs"]);

    let res = repo.check("staged");
    assert_exit(
        &res,
        0,
        "pre-existing (context) violation line must not be flagged",
    );
}

/// staged 模式：staged 删除违规行（`-` 行而非 `+` 行）不判定。
#[test]
fn test_staged_mode_ignores_deleted_lines() {
    let repo = TempRepo::new();
    let legacy_line = set_call("LEGACY");
    repo.write(
        "src/legacy.rs",
        &format!("{}\npub fn f() {{}}\n", legacy_line),
    );
    repo.commit_all();

    repo.write("src/legacy.rs", "pub fn f() {}\n");
    git(repo.path(), &["add", "src/legacy.rs"]);

    let res = repo.check("staged");
    assert_exit(
        &res,
        0,
        "staged deletion of a violation line must not be flagged",
    );
}

/// staged 模式：非 `*.rs` 文件不在扫描范围（保底一个干净 staged `.rs` 改动）。
#[test]
fn test_staged_mode_ignores_non_rs_files() {
    let repo = TempRepo::new();
    repo.write("src/clean.rs", "pub fn clean() {}\n");
    repo.commit_all();

    // 违规内容写进非 .rs 文件（不会被扫描），同时 stage 一个干净的 .rs 改动
    repo.write(
        "tools.sh",
        &format!("{}() {{\n  echo uses {}\n}}\n", BANNED_SET, BANNED_SET),
    );
    repo.write("src/clean.rs", "pub fn clean() {}\npub fn more() {}\n");
    git(repo.path(), &["add", "-A"]);

    let res = repo.check("staged");
    assert_exit(&res, 0, "non-.rs staged files must be out of scan scope");
}

/// 文案回归：失败输出携带正确引用，且不含过期章节引用与过期豁免路径。
#[test]
fn test_failure_message_has_no_stale_wording() {
    let repo = TempRepo::new();
    repo.write(
        "src/bad.rs",
        &format!("pub fn f() {{}}\n{}\n", set_call("A")),
    );
    repo.commit_all();

    let res = repo.check("all");
    assert_exit(&res, 1, "failure path drives the message regression check");
    let out = res.output();

    assert!(
        !out.contains(STALE_SECTION_REF),
        "stale section reference {STALE_SECTION_REF:?} must not appear, got:\n{out}"
    );
    assert!(
        !out.contains(STALE_EXEMPT_PATH),
        "stale exempt path {STALE_EXEMPT_PATH:?} must not appear, got:\n{out}"
    );
    assert!(
        out.contains(REF_STANDARDS),
        "expect reference {REF_STANDARDS}, got:\n{out}"
    );
    assert!(
        out.contains(REF_EXEMPT_PATH),
        "expect reference {REF_EXEMPT_PATH}, got:\n{out}"
    );
    assert!(
        out.contains(REF_CONTRIBUTING),
        "expect reference {REF_CONTRIBUTING}, got:\n{out}"
    );
}

/// 契约 ①：非法参数（未知模式 / 参数个数 > 1）→ exit 2，用法提示走 stderr。
#[test]
fn test_invalid_args_exit_2() {
    let repo = TempRepo::new();
    repo.write("src/lib.rs", "pub fn add() {}\n");
    repo.commit_all();

    let unknown = run_script(repo.path(), &["bogus"]);
    assert_exit(&unknown, 2, "unknown mode must exit 2");
    assert!(
        unknown.stderr.contains("usage:"),
        "usage hint must go to stderr, got:\n{}",
        unknown.output()
    );

    let too_many = run_script(repo.path(), &["all", "staged"]);
    assert_exit(&too_many, 2, "more than one argument must exit 2");
    assert!(
        too_many.stderr.contains("usage:"),
        "usage hint must go to stderr, got:\n{}",
        too_many.output()
    );
}

/// 契约 ①：非 git 目录（仓库守卫）→ exit 2，且报出守卫文案而非静默通过。
#[test]
fn test_non_git_dir_exits_2() {
    let dir = tempfile::TempDir::new().expect("create temp dir under /tmp");

    let res = run_script(dir.path(), &["all"]);
    assert_exit(&res, 2, "non-git directory must exit 2");
    let out = res.output();
    assert!(
        out.contains("必须在 git 仓库内运行"),
        "expect non-git guard message, got:\n{out}"
    );
}

/// 契约 ②：all 模式下 0 个 tracked `.rs`（空集合）→ exit 0 并输出 passed。
#[test]
fn test_all_mode_empty_tracked_rs_set_passes() {
    let repo = TempRepo::new();
    repo.write("README.md", "# fixture without any rust source\n");
    repo.commit_all();

    let res = repo.check("all");
    assert_exit(&res, 0, "zero tracked .rs files must pass");
    assert!(
        res.stdout.contains("env var check passed"),
        "expect passed marker in stdout, got:\n{}",
        res.output()
    );
}

/// 契约 ②：staged 模式下 index 为空（`git init` 后无 commit、无暂存）→ exit 0 并输出 passed。
#[test]
fn test_staged_mode_empty_index_passes() {
    let repo = TempRepo::new();

    let res = repo.check("staged");
    assert_exit(&res, 0, "empty staged index must pass");
    assert!(
        res.stdout.contains("env var check passed"),
        "expect passed marker in stdout, got:\n{}",
        res.output()
    );
}

/// 契约 ③：多 hunk 下 staged 行号准确——每个 `@@` 头重置新文件起始行号，
/// 违规落在第二及以后 hunk 时报出真实文件行号（不跨 hunk 连续累加）。
#[test]
fn test_staged_mode_multi_hunk_line_numbers() {
    let repo = TempRepo::new();
    let original: Vec<String> = (1..=10).map(|i| format!("pub fn f{i}() {{}}")).collect();
    repo.write("src/multi.rs", &format!("{}\n", original.join("\n")));
    repo.commit_all();

    // hunk 1：原第 1 行后插入干净行；hunk 2：原第 8 行后插入违规行（相距 7 行，独立成 hunk）
    let violation = set_call("MULTI");
    let mut edited = original.clone();
    edited.insert(1, "pub fn extra1() {}".to_string());
    edited.insert(9, violation.clone());
    repo.write("src/multi.rs", &format!("{}\n", edited.join("\n")));
    git(repo.path(), &["add", "src/multi.rs"]);

    let expected_ln = edited.iter().position(|l| *l == violation).unwrap() + 1;
    assert_eq!(
        expected_ln, 10,
        "fixture layout: violation must land in the 2nd hunk at new-file line 10"
    );

    let res = repo.check("staged");
    assert_exit(&res, 1, "violation in a later hunk must be flagged");
    let out = res.output();
    assert!(
        out.contains(&format!("src/multi.rs:{expected_ln}:{violation}")),
        "expect hit at src/multi.rs:{expected_ln} (per-@@ line reset), got:\n{out}"
    );
    assert!(
        !out.contains(&format!("src/multi.rs:3:{violation}")),
        "line number must reset per hunk, not accumulate across hunks, got:\n{out}"
    );
}

/// 回归 ①：内容行以 `++` + TAB 起始（diff 原始行 `+++` + TAB 形态）的 staged 新增行，
/// 与旧 hook 管道 `grep -v "^+++ "` 口径一致：不被文件头跳过条件吞掉，命中即报。
#[test]
fn test_staged_mode_flags_tab_form_content_line() {
    let repo = TempRepo::new();
    repo.write("src/base.rs", "pub fn base() {}\n");
    repo.commit_all();

    // 内容行 = `++` + TAB + 违规调用文本（raw diff 行以 `+++` + TAB 起始）
    let tab_line = format!("++\t{}", set_call("TAB"));
    repo.write("src/tab.rs", &format!("pub fn g() {{}}\n{}\n", tab_line));
    git(repo.path(), &["add", "src/tab.rs"]);

    let res = repo.check("staged");
    assert_exit(
        &res,
        1,
        "++\\t-prefixed content line must be flagged like old hook",
    );
    let out = res.output();
    assert!(
        out.contains(&format!("src/tab.rs:2:{tab_line}")),
        "expect hit at src/tab.rs:2, got:\n{out}"
    );
}

/// 回归 ②：被跳过的 `+++ ` 形态内容行（内容以 `++ ` 起始且含禁令 token，旧 hook 同口径
/// 不判）仍计入新文件行号——其后命中行报出的行号按实际文件行号推进，跳过行不参与判定。
#[test]
fn test_staged_mode_line_number_counts_skipped_header_form_line() {
    let repo = TempRepo::new();
    repo.write("src/base.rs", "pub fn base() {}\n");
    repo.commit_all();

    // 新文件三行：普通行 / `++ ` 起始且含 token 的内容行（raw `+++ `，同旧口径不判）/ 违规行
    let skipped = format!("++ // {} mentioned in a skipped-form line", BANNED_SET);
    let hit = set_call("AFTER_SKIPPED");
    repo.write(
        "src/skipped.rs",
        &format!("pub fn g() {{}}\n{}\n{}\n", skipped, hit),
    );
    git(repo.path(), &["add", "src/skipped.rs"]);

    let res = repo.check("staged");
    assert_exit(&res, 1, "hit after a skipped +++-form line must fail");
    let out = res.output();
    assert!(
        out.contains(&format!("src/skipped.rs:3:{hit}")),
        "expect hit at line 3 (skipped-form line counted), got:\n{out}"
    );
    assert!(
        !out.contains("src/skipped.rs:2:"),
        "+++ -form content line must not be judged (old-hook口径), got:\n{out}"
    );
}

/// 回归 ③：修改文件的 staged diff 含上下文行与删除行——命中行号按新文件实际行号计
/// （上下文行计入新文件并推进行号，删除行不计入）。
#[test]
fn test_staged_mode_line_number_counts_context_lines() {
    let repo = TempRepo::new();
    let original: Vec<String> = (1..=10).map(|i| format!("pub fn f{i}() {{}}")).collect();
    repo.write("src/ctx.rs", &format!("{}\n", original.join("\n")));
    repo.commit_all();

    // 替换第 3、8 行为违规：新文件中两处命中仍在第 3、8 行；其间散布上下文行与删除行
    let hit_a = set_call("CTX_A");
    let hit_b = set_call("CTX_B");
    let mut edited = original.clone();
    edited[2] = hit_a.clone();
    edited[7] = hit_b.clone();
    repo.write("src/ctx.rs", &format!("{}\n", edited.join("\n")));
    git(repo.path(), &["add", "src/ctx.rs"]);

    let res = repo.check("staged");
    assert_exit(&res, 1, "hits in a modified file must fail");
    let out = res.output();
    assert!(
        out.contains(&format!("src/ctx.rs:3:{hit_a}")),
        "expect hit at line 3, got:\n{out}"
    );
    assert!(
        out.contains(&format!("src/ctx.rs:8:{hit_b}")),
        "expect hit at line 8 (context lines before it count), got:\n{out}"
    );
}

/// 回归 ④：文件路径含禁令 token 时，git 文件头行（`+++ b/…`）不参与判定、不误报。
#[test]
fn test_staged_mode_header_with_token_in_path_not_flagged() {
    let repo = TempRepo::new();
    // 运行时拼装路径（如 src/<token>.rs）：源码自身不出现裸 token
    let rel = format!("src/{}.rs", BANNED_SET);
    repo.write(&rel, "pub fn a() {}\n");
    repo.commit_all();

    repo.write(&rel, "pub fn a() {}\npub fn b() {}\n");
    git(repo.path(), &["add", &rel]);

    let res = repo.check("staged");
    assert_exit(
        &res,
        0,
        "file header whose path contains the token must not be flagged",
    );
    let out = res.output();
    assert!(
        !out.contains(&rel),
        "path {rel} must not appear in output, got:\n{out}"
    );
    assert!(
        out.contains("env var check passed"),
        "expect passed marker in stdout, got:\n{out}"
    );
}
