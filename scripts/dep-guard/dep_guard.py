#!/usr/bin/env python3
"""Dep Guard – 解耦治理自动化守卫（STANDARDS.md 可脚本化规则的 CI 固化）。

Commands
--------
- ``check [--only <项>]``
    执行各项检查并汇总 PASS / FAIL / SKIP，退出码 = FAIL 项数。
    检查项：dep-edges（依赖方向）、second-exports（禁止二次出口）、
    common-admission（common 准入一致性）、dead-deps（死依赖）。

baseline 策略：baseline = 现状。baseline 内既有违规不报警，仅对
baseline 外新增越界报警 FAIL；baseline 只收窄不扩张，随各 P2 解耦
issue 收窄直至归零。
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path
from typing import Callable

import click

import allow_table
import baseline
import common_admission
import dead_deps
import dep_edges
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

CHECK_IDS = ("dep-edges", "second-exports", "common-admission", "dead-deps")

STATUS_PASS = "PASS"
STATUS_FAIL = "FAIL"
STATUS_SKIP = "SKIP"


class CheckResult:
    def __init__(self, check_id: str, status: str, lines: list[str]) -> None:
        self.check_id = check_id
        self.status = status
        self.lines = lines


def check_dep_edges(repo_root: Path) -> CheckResult:
    """依赖方向：workspace 内部常规依赖边 vs STANDARDS.md 允许边表。"""
    lines = ["[dep-edges] 依赖方向：workspace 内部常规依赖 vs 允许边表"]
    try:
        table = allow_table.load_allow_table(repo_root / "docs" / "design" / "STANDARDS.md")
        data = dep_edges.parse_metadata(dep_edges.cargo_metadata(repo_root))
    except (allow_table.AllowTableError, dep_edges.DepEdgesError) as exc:
        lines.append(f"  [FAIL] {exc}")
        return CheckResult("dep-edges", STATUS_FAIL, lines)

    violations = dep_edges.find_violations(data.edges, table, data.root_dir)
    actual = dep_edges.violation_items(violations, data.dir_to_pkg)
    base_items = baseline.load_baseline(BASELINE_DIR / "dep-edges.txt")
    diff = baseline.diff_baseline(base_items, actual)

    for item in diff.new_items:
        lines.append(f"  [FAIL] 新增越界边: {item}")
    for item in diff.eliminated_items:
        lines.append(f"  [info] 可收窄 baseline（越界边已消除）: {item}")
    status = STATUS_FAIL if diff.has_new else STATUS_PASS
    lines.append(
        f"  越界边 {len(actual)} 条 / baseline {len(base_items)} 条 / "
        f"新增 {len(diff.new_items)} / 已消除 {len(diff.eliminated_items)} → {status}"
    )
    return CheckResult("dep-edges", status, lines)


def check_second_exports(repo_root: Path) -> CheckResult:
    """禁止二次出口：common 项经本 crate 公共路径再暴露。"""
    lines = ["[second-exports] 禁止二次出口：common 项经公共路径再暴露"]
    try:
        data = dep_edges.parse_metadata(dep_edges.cargo_metadata(repo_root))
        items = second_export.find_second_exports(data)
    except (dep_edges.DepEdgesError, second_export.SecondExportError) as exc:
        lines.append(f"  [FAIL] {exc}")
        return CheckResult("second-exports", STATUS_FAIL, lines)

    base_items = baseline.load_baseline(BASELINE_DIR / "second-exports.txt")
    diff = baseline.diff_baseline(base_items, items)

    for item in diff.new_items:
        lines.append(f"  [FAIL] 新增二次出口: {item}")
    for item in diff.eliminated_items:
        lines.append(f"  [info] 可收窄 baseline（二次出口已消除）: {item}")
    status = STATUS_FAIL if diff.has_new else STATUS_PASS
    lines.append(
        f"  二次出口 {len(items)} 处 / baseline {len(base_items)} 条 / "
        f"新增 {len(diff.new_items)} / 已消除 {len(diff.eliminated_items)} → {status}"
    )
    return CheckResult("second-exports", status, lines)


def check_common_admission(repo_root: Path) -> CheckResult:
    """common 准入一致性：文档条目 ↔ common crate pub 项双向比对。"""
    lines = ["[common-admission] common 准入一致性：文档清单 ↔ common crate pub 项"]
    try:
        data = dep_edges.parse_metadata(dep_edges.cargo_metadata(repo_root))
        items = common_admission.find_deviations(repo_root, data)
    except (
        dep_edges.DepEdgesError,
        second_export.SecondExportError,
        common_admission.CommonAdmissionError,
    ) as exc:
        lines.append(f"  [FAIL] {exc}")
        return CheckResult("common-admission", STATUS_FAIL, lines)

    base_items = baseline.load_baseline(BASELINE_DIR / "common-admission.txt")
    diff = baseline.diff_baseline(base_items, items)

    for item in diff.new_items:
        lines.append(f"  [FAIL] 新增偏差: {item}")
    for item in diff.eliminated_items:
        lines.append(f"  [info] 可收窄 baseline（偏差已消除）: {item}")
    status = STATUS_FAIL if diff.has_new else STATUS_PASS
    lines.append(
        f"  偏差 {len(items)} 条 / baseline {len(base_items)} 条 / "
        f"新增 {len(diff.new_items)} / 已消除 {len(diff.eliminated_items)} → {status}"
    )
    return CheckResult("common-admission", status, lines)


def check_dead_deps(repo_root: Path) -> CheckResult:
    """死依赖：cargo machete 未使用依赖（覆盖外部 crate）。"""
    lines = ["[dead-deps] 死依赖：cargo machete 未使用依赖（含外部 crate）"]
    if not dead_deps.machete_available():
        lines.append("  [SKIP] cargo-machete 不可用（cargo machete --version 失败）")
        return CheckResult("dead-deps", STATUS_SKIP, lines)
    try:
        data = dead_deps.collect_unused(repo_root)
    except dead_deps.DeadDepsError as exc:
        lines.append(f"  [FAIL] {exc}")
        return CheckResult("dead-deps", STATUS_FAIL, lines)

    items = dead_deps.unused_items(data.unused)
    base_items = baseline.load_baseline(BASELINE_DIR / "dead-deps.txt")
    diff = baseline.diff_baseline(base_items, items)

    for item in diff.new_items:
        lines.append(f"  [FAIL] 新增死依赖: {item}")
    for item in diff.eliminated_items:
        lines.append(f"  [info] 可收窄 baseline（死依赖已消除）: {item}")
    status = STATUS_FAIL if diff.has_new else STATUS_PASS
    lines.append(
        f"  死依赖 {len(items)} 条 / baseline {len(base_items)} 条 / "
        f"新增 {len(diff.new_items)} / 已消除 {len(diff.eliminated_items)} "
        f"（machete {data.source} 解析）→ {status}"
    )
    return CheckResult("dead-deps", status, lines)


CHECK_RUNNERS: dict[str, Callable[[Path], CheckResult]] = {
    "dep-edges": check_dep_edges,
    "second-exports": check_second_exports,
    "common-admission": check_common_admission,
    "dead-deps": check_dead_deps,
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
def check_cmd(only: str | None) -> None:
    """执行检查并汇总；退出码 = FAIL 项数。"""
    results, fail_count = run_checks(REPO_ROOT, only)
    for result in results:
        for line in result.lines:
            print(line)
    print("汇总: " + " ".join(f"{result.check_id}={result.status}" for result in results))
    print(f"FAIL 项数: {fail_count}")
    # click group 的 standalone 模式不透传子命令返回值，须显式 ctx.exit。
    click.get_current_context().exit(fail_count)


if __name__ == "__main__":
    sys.exit(main())
