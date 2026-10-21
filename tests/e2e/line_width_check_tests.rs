//! 行为测试：`scripts/check-line-width.sh`（pre-commit 行宽硬限检查脚本）。
//!
//! 只驱动脚本的 `staged` 模式（含无参默认形态），不拉起完整 pre-commit hook
//! （hook 的文件行数上限、`cargo fmt`、角色规则不在本测试范围）。每个用例在
//! 独立的临时 git 仓库（`tempfile::TempDir`，落系统临时目录（跟随 $TMPDIR）、
//! Drop 自动清理）中执行，用例间无共享状态；全程无网络、无真实 LLM。
//!
//! 脚本驱动与临时仓库基础设施复用共享 helper
//! [`super::helpers::check_script`]（STANDARDS §10：共享 helper 提取为公共
//! 模块，不在单个 case 内重复内嵌）；本文件只保留本脚本特有的常量与夹具
//! 生成函数。
//!
//! 归档依据（docs/developer/STANDARDS.md）：§1 spawn 独立脚本进程 → e2e 档；
//! §2/§3 `tests/e2e/` 单 binary + 复数 `_tests.rs` 命名；§8 临时文件走 TempDir。
//!
//! 口径提示：行宽按字符（Unicode code point）计、非字节；.rs 与 Cargo.toml
//! 均查全部新增行；只判定 staged 新增（+）行，存量超宽行不在增量口径内。

use std::path::Path;

use super::helpers::check_script::{assert_exit, assert_passed_marker, CheckScript};

/// 行宽上限：与脚本实现和 CONTRIBUTING.md「硬性限制」保持一致。
const MAX_WIDTH: usize = 100;

/// 本套件脚本通过路径输出的 passed 标记（与脚本实现一致，断言时参数化传入）。
const PASSED_MARKER: &str = "line width check passed";

/// 本套件共用脚本驱动器：`scripts/check-line-width.sh` + fixture 提交身份
/// （`user.name=line-width-check-tests` /
/// `user.email=line-width-check-tests@example.invalid`，与提取前的本地实现
/// 逐字一致），全部用例共用同一构造。
fn script() -> CheckScript {
    CheckScript::new("check-line-width.sh", "line-width-check-tests")
}

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

/// 夹具：总宽恰为 `width` 字符的字符串字面量代码行（非注释；rustfmt 不拆
/// 字符串字面量，超宽时无 fmt 守卫，由本脚本拦截——原口径的守卫盲区）。
fn code_line(width: usize) -> String {
    let prefix = "pub const S: &str = \"";
    assert!(width > prefix.len() + 2, "fixture width too small");
    format!("{prefix}{}\";", "a".repeat(width - prefix.len() - 2))
}

/// 夹具：总宽恰为 `width` 字符的普通代码行（非注释代码行形态）。
/// 非注释新增行无论 fmt 可折与否均由本脚本拦截，本夹具只代表
/// 非注释代码行形态。
fn plain_code_line(width: usize) -> String {
    let prefix = "let _ = f(";
    let suffix = ");";
    assert!(
        width > prefix.len() + suffix.len(),
        "fixture width too small"
    );
    format!(
        "{prefix}{}{suffix}",
        "a".repeat(width - prefix.len() - suffix.len())
    )
}

/// 夹具：`// ` + `n_chars` 个中文字符的注释行（总宽 3 + n_chars 字符，1 字符 3 字节）。
fn chinese_comment(n_chars: usize) -> String {
    format!("// {}", "中".repeat(n_chars))
}

/// 正常路径：`git init` 后无任何 staged 目标文件（空 index）→ exit 0 并输出 passed。
/// 同时钉住无参调用形态（默认模式 = staged）。
#[test]
fn test_line_width_empty_index_passes() {
    let script = script();
    let repo = script.temp_repo();

    let res = script.run(repo.path(), &[]);
    assert_exit(&res, 0, "empty staged index must pass via default mode");
    assert_passed_marker(&res, PASSED_MARKER, "empty staged index");
}

/// 正常路径 + 边界：staged 的非目标文件（.md，含 150 字符超宽行）不在扫描范围。
#[test]
fn test_line_width_non_target_file_passes() {
    let script = script();
    let repo = script.temp_repo();
    let long = "m".repeat(150);
    repo.write("notes.md", &format!("# notes\n{long}\n"));
    repo.stage("notes.md");

    let res = script.run(repo.path(), &["staged"]);
    assert_exit(&res, 0, "staged non-target file must be out of scan scope");
    assert_passed_marker(&res, PASSED_MARKER, "non-target staged file");
}

/// 正常路径 + 边界：恰 100 字符（上限值）的注释行、普通代码行、字符串字面量
/// 行与 Cargo.toml 行均放行。
#[test]
fn test_line_width_exactly_100_passes() {
    let script = script();
    let repo = script.temp_repo();
    repo.write(
        "src/edge.rs",
        &format!(
            "pub fn f() {{}}\n{}\n{}\n{}\n",
            comment_line("// ", MAX_WIDTH),
            plain_code_line(MAX_WIDTH),
            code_line(MAX_WIDTH)
        ),
    );
    repo.write(
        "Cargo.toml",
        &format!("[package]\n{}\n", toml_line(MAX_WIDTH)),
    );
    repo.stage("src/edge.rs");
    repo.stage("Cargo.toml");

    let res = script.run(repo.path(), &["staged"]);
    assert_exit(&res, 0, "exactly 100-char lines must pass");
    assert_passed_marker(&res, PASSED_MARKER, "boundary 100-char lines");
}

/// 错误路径：101 字符 `//` 注释行拦截，文案含 文件:行号:长度 与修复指引。
#[test]
fn test_line_width_flags_101_slash_comment() {
    let script = script();
    let repo = script.temp_repo();
    let line = comment_line("// ", MAX_WIDTH + 1);
    repo.write("src/bad.rs", &format!("pub fn f() {{}}\n{line}\n"));
    repo.stage("src/bad.rs");

    let res = script.run(repo.path(), &["staged"]);
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
    let script = script();
    let repo = script.temp_repo();
    let doc = comment_line("/// ", MAX_WIDTH + 1);
    let inner = comment_line("//! ", MAX_WIDTH + 2);
    repo.write("src/docs.rs", &format!("{doc}\n{inner}\n"));
    repo.stage("src/docs.rs");

    let res = script.run(repo.path(), &["staged"]);
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
    let script = script();
    let repo = script.temp_repo();
    repo.write(
        "crates/app/Cargo.toml",
        &format!("[dependencies]\n{}\n", toml_line(MAX_WIDTH + 1)),
    );
    repo.stage("crates/app/Cargo.toml");

    let res = script.run(repo.path(), &["staged"]);
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
    let script = script();
    let repo = script.temp_repo();
    // 4 空格缩进 + 97 字符注释 = 101 字符
    let line = format!("    {}", comment_line("// ", MAX_WIDTH - 3));
    assert_eq!(line.chars().count(), MAX_WIDTH + 1, "fixture layout");
    repo.write("src/indent.rs", &format!("pub fn f() {{}}\n{line}\n"));
    repo.stage("src/indent.rs");

    let res = script.run(repo.path(), &["staged"]);
    assert_exit(&res, 1, "indented overwidth comment must be flagged");
    let out = res.output();
    assert!(
        out.contains(&format!("src/indent.rs:2:{}", MAX_WIDTH + 1)),
        "expect location src/indent.rs:2:101, got:\n{out}"
    );
}

/// 错误路径：staged 新增的非注释超宽代码行被拦（新口径：.rs 查全部新增行，
/// 不再豁免非注释代码行）。
#[test]
fn test_line_width_flags_overwidth_code_line() {
    let script = script();
    let repo = script.temp_repo();
    let line = plain_code_line(MAX_WIDTH + 20);
    repo.write("src/long_code.rs", &format!("{line}\n"));
    repo.stage("src/long_code.rs");

    let res = script.run(repo.path(), &["staged"]);
    assert_exit(
        &res,
        1,
        "overwidth staged non-comment code line must be flagged",
    );
    let out = res.output();
    assert!(
        out.contains(&format!("src/long_code.rs:1:{}", MAX_WIDTH + 20)),
        "expect location src/long_code.rs:1:120, got:\n{out}"
    );
    assert!(
        out.contains("修复指引"),
        "expect fix guidance in message, got:\n{out}"
    );
}

/// 错误路径 + 漏洞回归钉子：staged 新增的超宽字符串字面量行被拦（rustfmt 不拆
/// 字符串字面量，原口径「.rs 只查注释行」对该类行无任何守卫）。
#[test]
fn test_line_width_flags_overwidth_string_literal_line() {
    let script = script();
    let repo = script.temp_repo();
    let line = code_line(MAX_WIDTH + 1);
    repo.write("src/lit.rs", &format!("pub fn f() {{}}\n{line}\n"));
    repo.stage("src/lit.rs");

    let res = script.run(repo.path(), &["staged"]);
    assert_exit(
        &res,
        1,
        "overwidth staged string literal line must be flagged",
    );
    let out = res.output();
    assert!(
        out.contains(&format!("src/lit.rs:2:{}", MAX_WIDTH + 1)),
        "expect location src/lit.rs:2:101, got:\n{out}"
    );
}

/// 增量口径：存量（HEAD 既有）超宽注释行不在 staged 新增行内 → 放行。
#[test]
fn test_line_width_legacy_line_passes() {
    let script = script();
    let repo = script.temp_repo();
    let legacy = comment_line("// ", MAX_WIDTH + 1);
    repo.write("src/legacy.rs", &format!("{legacy}\npub fn old() {{}}\n"));
    repo.commit_all();

    // 只在文件末尾追加干净行并 stage：存量超宽行在 diff 中为上下文行，不判定
    repo.write(
        "src/legacy.rs",
        &format!("{legacy}\npub fn old() {{}}\npub fn new() {{}}\n"),
    );
    repo.stage("src/legacy.rs");

    let res = script.run(repo.path(), &["staged"]);
    assert_exit(&res, 0, "legacy (context) overwidth line must pass");
    assert_passed_marker(&res, PASSED_MARKER, "legacy overwidth line untouched");
}

/// 增量口径 + 新口径边界：存量（HEAD 既有）超宽字符串字面量行未被本轮触碰时
/// 同样放行（新口径只拦新增行，不背负历史技术债）。
#[test]
fn test_line_width_legacy_overwidth_code_line_passes() {
    let script = script();
    let repo = script.temp_repo();
    let legacy = code_line(MAX_WIDTH + 20);
    repo.write(
        "src/legacy_code.rs",
        &format!("{legacy}\npub fn old() {{}}\n"),
    );
    repo.commit_all();

    // 只在文件末尾追加干净行并 stage：存量超宽行在 diff 中为上下文行，不判定
    repo.write(
        "src/legacy_code.rs",
        &format!("{legacy}\npub fn old() {{}}\npub fn new() {{}}\n"),
    );
    repo.stage("src/legacy_code.rs");

    let res = script.run(repo.path(), &["staged"]);
    assert_exit(
        &res,
        0,
        "legacy overwidth code line must pass when untouched",
    );
    assert_passed_marker(&res, PASSED_MARKER, "legacy overwidth code line untouched");
}

/// 边界：中文注释行按字符计——40 中文字符（123 字节）通过；>100 字符中文行拦截。
#[test]
fn test_line_width_chinese_counted_by_chars() {
    let script = script();
    let repo = script.temp_repo();

    // 43 字符 / 123 字节：字节口径会误拦（123 > 100），字符口径必须放行
    let ok = chinese_comment(40);
    assert_eq!(ok.chars().count(), 43, "fixture: 3 prefix + 40 chars");
    assert_eq!(ok.len(), 123, "fixture: 3 ascii bytes + 40 * 3 bytes");
    repo.write("src/cn_ok.rs", &format!("pub fn f() {{}}\n{ok}\n"));
    repo.stage("src/cn_ok.rs");
    let res = script.run(repo.path(), &["staged"]);
    assert_exit(&res, 0, "40-Chinese-char comment must pass by char count");
    assert_passed_marker(&res, PASSED_MARKER, "chinese comment within 100 chars");

    // 103 字符：超上限 → 拦截（字符口径，非字节放大）
    let over = chinese_comment(MAX_WIDTH);
    assert_eq!(over.chars().count(), 103, "fixture: 3 prefix + 100 chars");
    repo.write("src/cn_over.rs", &format!("pub fn f() {{}}\n{over}\n"));
    repo.stage("src/cn_over.rs");
    let res = script.run(repo.path(), &["staged"]);
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
    let script = script();
    let repo = script.temp_repo();
    let bad = comment_line("// ", MAX_WIDTH + 1);
    repo.write("src/fix.rs", &format!("pub fn f() {{}}\n{bad}\n"));
    repo.stage("src/fix.rs");
    let res = script.run(repo.path(), &["staged"]);
    assert_exit(&res, 1, "overwidth staged comment must be flagged first");

    let fixed = comment_line("// ", MAX_WIDTH);
    repo.write("src/fix.rs", &format!("pub fn f() {{}}\n{fixed}\n"));
    repo.stage("src/fix.rs");
    let res = script.run(repo.path(), &["staged"]);
    assert_exit(&res, 0, "restaged fixed file must pass");
    assert_passed_marker(&res, PASSED_MARKER, "restaged fixed file");
}

/// 边界 + 行号口径：多 hunk 场景下第二 hunk 中超宽注释行的报出行号正确
/// （逐 @@ 独立记账，不跨 hunk 累加）。
///
/// 夹具：基线 15 行短代码行提交后，在 f02 之后插入超宽注释 A（新文件第 3 行）、
/// 在 f13 之后插入超宽注释 B（新文件第 15 行）。两处改动间隔 > 2×默认上下文，
/// diff 拆为两个 hunk；若行号跨 hunk 累加，第二 hunk 的报出行号必然错位。
#[test]
fn test_line_width_multi_hunk_second_hunk_line_number() {
    let script = script();
    let repo = script.temp_repo();
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

    let res = script.run(repo.path(), &["staged"]);
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
    let script = script();
    let repo = script.temp_repo();

    let unknown = script.run(repo.path(), &["bogus"]);
    assert_exit(&unknown, 2, "unknown mode must exit 2");
    assert!(
        unknown.stderr.contains("usage:"),
        "usage hint must go to stderr, got:\n{}",
        unknown.output()
    );

    let too_many = script.run(repo.path(), &["staged", "staged"]);
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
    let script = script();
    let dir = tempfile::TempDir::new().expect("create temp dir under system temp dir");

    let res = script.run_with_env(dir.path(), &["staged"], |cmd| {
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
