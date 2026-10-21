#!/usr/bin/env python3
"""Dep Guard – 解耦治理自动化守卫（STANDARDS.md 可脚本化规则的 CI 固化）。

Commands
--------
- ``check [--only <项>] [--fix]``
    执行各项检查并汇总 PASS / FAIL / SKIP，退出码 = FAIL 项数。
    检查项：dep-edges（依赖方向）、second-exports（禁止二次出口）、
    common-admission（common 准入一致性）、dead-deps（死依赖）、
    rust-version-declared（rust-version 声明完整性）。
    ``--fix``：对 FIXABLE 四项（不含 rust-version-declared）自动删除
    baseline 中已消除的条目并回写 baseline 文件；新增偏差照常 FAIL，
    退出码不变。

baseline 策略：baseline = 现状。baseline 内既有违规不报警，仅对
baseline 外新增越界报警 FAIL；baseline 只收窄不扩张，随各 P2 解耦
issue 收窄直至归零。
"""

from __future__ import annotations

import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

import click

import allow_table
import baseline
import common_admission
import dead_deps
import dep_edges
import rust_version
import second_export

SCRIPT_DIR = Path(__file__).parent.resolve()
REPO_ROOT = Path(
    subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=SCRIPT_DIR,
        text=True,
    ).strip()
)
BASELINE_DIR = SCRIPT_DIR / "baselines"

CHECK_IDS = (
    "dep-edges",
    "second-exports",
    "common-admission",
    "dead-deps",
    "rust-version-declared",
)

# check --fix 收窄范围：仅四项 baseline 参与（rust-version-declared 不参与）。
FIXABLE_CHECKS = frozenset(
    {"dep-edges", "second-exports", "common-admission", "dead-deps"}
)

STATUS_PASS = "PASS"
STATUS_FAIL = "FAIL"
STATUS_SKIP = "SKIP"


@dataclass(frozen=True)
class CheckResult:
    """单项检查结果：标识 + 状态 + 报文行（全链路只读，构造即定稿）。

    后三个字段供 ``check --fix`` 收窄编排消费：SKIP 段与错误路径不携带
    （默认空 / None），apply_fix 自动跳过；eliminated_items 保持 baseline
    原始条目（second-exports 含旧行号），回写按原始条目精确匹配。
    """

    check_id: str  # 检查项标识，取值见 CHECK_IDS
    status: str  # PASS / FAIL / SKIP（STATUS_* 常量）
    lines: list[str]  # 报文行（首行标题 + FAIL/info 条目 + 汇总行）
    eliminated_items: list[str] = field(default_factory=list)  # 已消除条目
    baseline_path: Path | None = None  # baseline 文件路径（SKIP / 错误路径为 None）
    narrowed_label: str | None = None  # [fix] 措辞标签（对齐 spec.narrowed_label）


@dataclass(frozen=True)
class DiffReportSpec:
    """check_* 报文模板的措辞参数（收敛五份重复拼装的静态差异）。"""

    check_id: str
    fail_label: str  # FAIL 新增条目措辞，如「新增越界边」
    narrowed_label: str  # info 可收窄条目措辞，如「越界边已消除」
    count_noun: str  # 汇总行计数对象名，如「越界边」
    count_word: str = "条"  # 汇总行计数词（条 / 处）；baseline 段恒为条（保持输出不变）
    summary_suffix: str = ""  # 汇总行后缀，如「（machete … 解析）」


def report_error(lines: list[str], check_id: str, exc: Exception) -> CheckResult:
    """异常早退路径公共模板：追加 `[FAIL] {exc}` 并返回 FAIL 结果。"""
    lines.append(f"  [FAIL] {exc}")
    return CheckResult(check_id, STATUS_FAIL, lines)


def report_diff(
    lines: list[str],
    spec: DiffReportSpec,
    diff: baseline.BaselineDiff,
    total_count: int,
    baseline_count: int,
    reason_by_path: dict[str, str] | None = None,
) -> CheckResult:
    """baseline diff 报文公共模板：FAIL 新增 / info 可收窄 / 汇总行 → CheckResult。

    措辞差异经 ``DiffReportSpec`` 参数化；reason 注入（如 rust-version 的
    违规原因，不入 diff 仅入报文）经 ``reason_by_path`` 提供，条目缺
    reason 时回退基础格式；baseline 归一化 key 由调用方在
    ``baseline.diff_baseline`` 处选择。状态判定固定为：
    diff 有新增 → FAIL，否则 PASS。baseline 路径按 ``spec.check_id`` 推导
    （``BASELINE_DIR / f"{check_id}.txt"``）随结果携带，供 ``check --fix``
    收窄回写；rust-version-declared 的豁免由 ``FIXABLE_CHECKS`` 把关。
    """
    reasons = reason_by_path or {}
    for item in diff.new_items:
        reason = reasons.get(item)
        if reason is not None:
            lines.append(f"  [FAIL] {spec.fail_label}: {item}（{reason}）")
        else:
            lines.append(f"  [FAIL] {spec.fail_label}: {item}")
    for item in diff.eliminated_items:
        lines.append(f"  [info] 可收窄 baseline（{spec.narrowed_label}）: {item}")
    status = STATUS_FAIL if diff.has_new else STATUS_PASS
    lines.append(
        f"  {spec.count_noun} {total_count} {spec.count_word} / baseline {baseline_count} 条 / "
        f"新增 {len(diff.new_items)} / 已消除 {len(diff.eliminated_items)} "
        f"{spec.summary_suffix}→ {status}"
    )
    return CheckResult(
        spec.check_id,
        status,
        lines,
        eliminated_items=diff.eliminated_items,
        baseline_path=BASELINE_DIR / f"{spec.check_id}.txt",
        narrowed_label=spec.narrowed_label,
    )


def check_dep_edges(repo_root: Path) -> CheckResult:
    """依赖方向：workspace 内部常规依赖边 vs STANDARDS.md 允许边表。"""
    lines = ["[dep-edges] 依赖方向：workspace 内部常规依赖 vs 允许边表"]
    spec = DiffReportSpec(
        check_id="dep-edges",
        fail_label="新增越界边",
        narrowed_label="越界边已消除",
        count_noun="越界边",
    )
    try:
        table = allow_table.load_allow_table(repo_root / "docs" / "design" / "STANDARDS.md")
        data = dep_edges.parse_metadata(dep_edges.cargo_metadata(repo_root))
    except (allow_table.AllowTableError, dep_edges.DepEdgesError) as exc:
        return report_error(lines, spec.check_id, exc)

    violations = dep_edges.find_violations(data.edges, table, data.root_dir)
    actual = dep_edges.violation_items(violations, data.dir_to_pkg)
    base_items = baseline.load_baseline(BASELINE_DIR / f"{spec.check_id}.txt")
    diff = baseline.diff_baseline(base_items, actual)
    return report_diff(lines, spec, diff, len(actual), len(base_items))


def check_second_exports(repo_root: Path) -> CheckResult:
    """禁止二次出口：common 项经本 crate 公共路径再暴露。"""
    lines = ["[second-exports] 禁止二次出口：common 项经公共路径再暴露"]
    spec = DiffReportSpec(
        check_id="second-exports",
        fail_label="新增二次出口",
        narrowed_label="二次出口已消除",
        count_noun="二次出口",
        count_word="处",
    )
    try:
        data = dep_edges.parse_metadata(dep_edges.cargo_metadata(repo_root))
        items = second_export.find_second_exports(data)
    except (dep_edges.DepEdgesError, second_export.SecondExportError) as exc:
        return report_error(lines, spec.check_id, exc)

    base_items = baseline.load_baseline(BASELINE_DIR / f"{spec.check_id}.txt")
    # second-exports 条目含行号：diff 判定键去行号归一化，行号仅作定位信息，
    # 代码移动导致的行号漂移不再触发「新增」FAIL（报告仍返回原始条目）。
    diff = baseline.diff_baseline(base_items, items, key=second_export.strip_lineno)
    return report_diff(lines, spec, diff, len(items), len(base_items))


def check_common_admission(repo_root: Path) -> CheckResult:
    """common 准入一致性：文档条目 ↔ common crate pub 项双向比对。"""
    lines = ["[common-admission] common 准入一致性：文档清单 ↔ common crate pub 项"]
    spec = DiffReportSpec(
        check_id="common-admission",
        fail_label="新增偏差",
        narrowed_label="偏差已消除",
        count_noun="偏差",
    )
    try:
        data = dep_edges.parse_metadata(dep_edges.cargo_metadata(repo_root))
        items = common_admission.find_deviations(repo_root, data)
    except (
        dep_edges.DepEdgesError,
        second_export.SecondExportError,
        common_admission.CommonAdmissionError,
    ) as exc:
        return report_error(lines, spec.check_id, exc)

    base_items = baseline.load_baseline(BASELINE_DIR / f"{spec.check_id}.txt")
    diff = baseline.diff_baseline(base_items, items)
    return report_diff(lines, spec, diff, len(items), len(base_items))


def check_dead_deps(repo_root: Path) -> CheckResult:
    """死依赖：cargo machete 未使用依赖（覆盖外部 crate）。"""
    check_id = "dead-deps"
    lines = ["[dead-deps] 死依赖：cargo machete 未使用依赖（含外部 crate）"]
    if not dead_deps.machete_available():
        lines.append("  [SKIP] cargo-machete 不可用（cargo machete --version 失败）")
        return CheckResult(check_id, STATUS_SKIP, lines)
    try:
        data = dead_deps.collect_unused(repo_root)
    except dead_deps.DeadDepsError as exc:
        return report_error(lines, check_id, exc)

    # spec 后置构造：summary_suffix 需 data.source（machete 解析来源）。
    spec = DiffReportSpec(
        check_id=check_id,
        fail_label="新增死依赖",
        narrowed_label="死依赖已消除",
        count_noun="死依赖",
        summary_suffix=f"（machete {data.source} 解析）",
    )
    items = dead_deps.unused_items(data.unused)
    base_items = baseline.load_baseline(BASELINE_DIR / f"{check_id}.txt")
    diff = baseline.diff_baseline(base_items, items)
    return report_diff(lines, spec, diff, len(items), len(base_items))


def check_rust_version_declared(repo_root: Path) -> CheckResult:
    """rust-version 声明完整性：workspace 成员须继承 workspace MSRV。"""
    lines = [
        "[rust-version-declared] rust-version 声明完整性：成员须 rust-version.workspace = true"
    ]
    spec = DiffReportSpec(
        check_id="rust-version-declared",
        fail_label="新增违规",
        narrowed_label="rust-version 已声明",
        count_noun="违规",
    )
    try:
        violations = rust_version.find_violations(repo_root)
    except rust_version.RustVersionError as exc:
        return report_error(lines, spec.check_id, exc)

    items = [path for path, _ in violations]
    base_items = baseline.load_baseline(BASELINE_DIR / "rust-version-declared.txt")
    # 条目即成员相对路径，默认按条目整体比较；原因不参与比较，仅用于报文。
    diff = baseline.diff_baseline(base_items, items)
    return report_diff(
        lines, spec, diff, len(items), len(base_items), reason_by_path=dict(violations)
    )


CHECK_RUNNERS: dict[str, Callable[[Path], CheckResult]] = {
    "dep-edges": check_dep_edges,
    "second-exports": check_second_exports,
    "common-admission": check_common_admission,
    "dead-deps": check_dead_deps,
    "rust-version-declared": check_rust_version_declared,
}


def run_checks(repo_root: Path, only: str | None) -> tuple[list[CheckResult], int]:
    """执行选中的检查项；未接入的项记 SKIP。返回结果与 FAIL 项数。"""
    selected = [only] if only else list(CHECK_IDS)
    results = []
    for check_id in selected:
        runner = CHECK_RUNNERS.get(check_id)
        if runner is None:
            results.append(
                CheckResult(check_id, STATUS_SKIP, [f"[{check_id}] SKIP（未接入）"])
            )
        else:
            results.append(runner(repo_root))
    fail_count = sum(1 for result in results if result.status == STATUS_FAIL)
    return results, fail_count


def apply_fix(result: CheckResult, should_fix: bool) -> list[str]:
    """``--fix`` 收窄编排：删除 FIXABLE 检查 baseline 中已消除条目，返回报文行。

    仅 should_fix 开启、检查项属 FIXABLE_CHECKS、结果非 SKIP 且携带
    baseline 路径与已消除条目时执行（SKIP 段显式跳过，
    rust-version-declared 由 FIXABLE_CHECKS 排除）；与 FAIL 正交，
    新增偏差照常报警、退出码不受影响。每实际删除一条返回一行
    ``[fix] 已收窄 baseline（…）: <item>``；条目在文件中已不存在时
    remove_entries no-op，不报错。
    """
    if (
        not should_fix
        or result.status == STATUS_SKIP
        or result.check_id not in FIXABLE_CHECKS
        or result.baseline_path is None
        or not result.eliminated_items
    ):
        return []
    removed = baseline.remove_entries(result.baseline_path, result.eliminated_items)
    return [
        f"  [fix] 已收窄 baseline（{result.narrowed_label}）: {item}"
        for item in removed
    ]


@click.group()
def main() -> int:
    """Dep Guard – 解耦治理自动化守卫。"""
    return 0


@main.command(name="check")
@click.option(
    "--only",
    type=click.Choice(CHECK_IDS),
    default=None,
    help="只执行指定检查项（默认全部）。",
)
@click.option(
    "--fix",
    "should_fix",
    is_flag=True,
    default=False,
    help=(
        "自动收窄 baseline：删除 FIXABLE 四项（dep-edges / second-exports / "
        "common-admission / dead-deps）已消除的条目并回写 baseline 文件；"
        "rust-version-declared 不参与。新增偏差照常 FAIL，退出码不变。"
    ),
)
def check_cmd(only: str | None, should_fix: bool) -> None:
    """执行检查并汇总；退出码 = FAIL 项数。--fix 额外收窄 baseline 已消除条目。"""
    results, fail_count = run_checks(REPO_ROOT, only)
    for result in results:
        for line in result.lines:
            print(line)
        for fix_line in apply_fix(result, should_fix):
            print(fix_line)
    print("汇总: " + " ".join(f"{result.check_id}={result.status}" for result in results))
    print(f"FAIL 项数: {fail_count}")
    # click group 的 standalone 模式不透传子命令返回值，须显式 ctx.exit。
    click.get_current_context().exit(fail_count)


if __name__ == "__main__":
    sys.exit(main())
