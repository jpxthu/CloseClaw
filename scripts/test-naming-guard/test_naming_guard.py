#!/usr/bin/env python3
"""test-naming-guard — 测试函数 test_ 前缀静态守卫（STANDARDS.md §3 的 CI 固化）。

多行状态机扫描仓库 `.rs` 文件：识别 `#[test]` / `#[tokio::test]` 属性行（含带参
形态如 `#[tokio::test(flavor = "current_thread")]` 与折行 `(` 结尾；`#[cfg(test)]` /
`#[test_case]` / `#[test_suite]` 均不算——排除边界同 ``scripts/code-stats/collect_code_stats.py``
的 ``TEST_ATTR_RE``，属性行 ``test`` 词后内容不限，故比其更宽），向后跳过连续属性行
（如 `#[should_panic]`）取第一个 ``(async )?fn`` 声明行，函数名缺 `test_` 前缀即违规。
扫描按测试属性定位、不按文件名猜：全仓 `.rs`（`src/`、`tests/`、`crates/*/src`、
`crates/*/tests`），排除 `target/` 与隐藏目录。

baseline 策略（dep-guard 同款）：baseline = 现状。baseline 内既有违规豁免，仅对
baseline 外新增违规报警 FAIL；比对键去行号归一化（``strip_key``），行号漂移不触发
FAIL；已消除条目提示可收窄（不 FAIL），``--fix`` 回写删除，``--update-baseline``
全量重写。条目含行号仅作定位信息；同键多条目（同文件同名函数）报一条代表。
"""

from __future__ import annotations

import re
import subprocess
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

import click

SCRIPT_DIR = Path(__file__).parent.resolve()
BASELINE_PATH = SCRIPT_DIR / "baselines" / "missing_test_prefix.txt"
RULE_LABEL = "missing_test_prefix"

# ---------- 测试属性识别（collect_code_stats.py TEST_ATTR_RE 的放宽版）----------

# 匹配（对 strip 后的行；``test`` 词边界后允许任意内容，兼容带参与尾注释）：
#   #[test]                                   - 是
#   #[test (flavor = "x")]                    - 是（带括号参数）
#   #[tokio::test(flavor = "current_thread")] - 是（单行带参）
#   #[test] // comment                        - 是（属性行尾注释）
#   #[test                                    - 是（属性行折行、内容跟随）
#   #[tokio::test(                            - 是（折行 `(` 结尾、参数跟随）
#   #[tokio::test]                            - 是
#   #[async_std::test]                        - 是
#   #[test_suite]                             - 否（`_` 是 word char，挡住 \b）
#   #[test_case] / #[test_case(1)]            - 否（同上）
#   #[cfg(test)]                              - 否（`#\[` 后不是 test）
TEST_ATTR_RE = re.compile(r"^#\[\s*(?:\w+::)*test\b.*$")

# fn 声明行：可选可见性（pub / pub(...)）+ 可选 async + fn + 函数名
FN_DECL_RE = re.compile(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_]\w*)")

# baseline 条目行号分隔符（首现 `:数字: `），供去行号归一化
_LINENO_SEP_RE = re.compile(r"^(.+?):\d+: ")


@dataclass
class BaselineDiff:
    """三态比对结果：新增（FAIL）/ 已消除（可收窄）。剩余为豁免。"""

    new_items: list[str]
    eliminated_items: list[str]

    @property
    def has_new(self) -> bool:
        return bool(self.new_items)


# ---------- 扫描器（多行状态机）-------------------------------------------------


def scan_file(path: Path) -> list[tuple[int, str]]:
    """单文件状态机：返回违规 ``[(fn 声明行号, 函数名), ...]``。

    识别属性行后进入 seek 态：空行 / 注释 / 连续属性行跳过；首个 ``fn`` 声明
    行定谳（缺前缀记违规）并退出 seek 态；其余任何行（如 `}`、普通语句）同样
    退出 seek 态（保守方向：不向后远距离误配无关 fn）；EOF 自然结束不崩溃。
    """
    violations: list[tuple[int, str]] = []
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError):
        return []
    seek_fn = False
    for lineno, line in enumerate(lines, start=1):
        stripped = line.strip()
        if seek_fn:
            if not stripped or stripped.startswith("//") or stripped.startswith("#["):
                continue
            match = FN_DECL_RE.match(line)
            if match is not None:
                fn_name = match.group(1)
                if not fn_name.startswith("test_"):
                    violations.append((lineno, fn_name))
            seek_fn = False
        elif TEST_ATTR_RE.match(stripped):
            seek_fn = True
    return violations


def find_rs_files(root: Path) -> list[Path]:
    """仓库 `.rs` 文件清单：排除 `target/` 与隐藏目录（按路径任意层级剪枝）。"""
    return [
        path
        for path in sorted(root.rglob("*.rs"))
        if not any(
            part == "target" or part.startswith(".")
            for part in path.relative_to(root).parts[:-1]
        )
    ]


def scan_root(root: Path) -> list[str]:
    """全量扫描：返回排序条目 ``相对路径:行号: missing_test_prefix(函数名)``。"""
    entries: list[str] = []
    for path in find_rs_files(root):
        rel = path.relative_to(root).as_posix()
        for lineno, fn_name in scan_file(path):
            entries.append(f"{rel}:{lineno}: {RULE_LABEL}({fn_name})")
    return sorted(entries)


# ---------- baseline 引擎（仿 dep-guard/baseline.py，比对键去行号）---------------


def strip_key(entry: str) -> str:
    """比对键：去掉首个行号段（``路径:行号: 规则`` → ``路径: 规则``）。

    代码移动导致的行号漂移不再触发「新增」FAIL，也不误报「已消除」；
    函数名 / 文件变化仍会被正确识别（真新增 / 真消除）。
    """
    return _LINENO_SEP_RE.sub(r"\1: ", entry, count=1)


def diff_baseline(
    baseline_items: list[str],
    actual_items: list[str],
    key=strip_key,
) -> BaselineDiff:
    """多重集三态比对：实际 − baseline = 新增（FAIL）；baseline − 实际 = 已消除。

    与 dep-guard 的 dict 覆盖语义不同，这里按 key 计数（多重集）差集：同键
    多条目并存不丢条目；报告每键返回一条代表条目（原始形态，保留行号定位）。
    """
    base_counts = Counter(key(item) for item in baseline_items)
    actual_counts = Counter(key(item) for item in actual_items)
    new_keys = actual_counts - base_counts
    eliminated_keys = base_counts - actual_counts
    base_repr: dict[str, str] = {}
    actual_repr: dict[str, str] = {}
    for item in baseline_items:
        base_repr.setdefault(key(item), item)
    for item in actual_items:
        actual_repr.setdefault(key(item), item)
    return BaselineDiff(
        new_items=sorted(actual_repr[k] for k in new_keys),
        eliminated_items=sorted(base_repr[k] for k in eliminated_keys),
    )


def load_baseline(path: Path) -> list[str]:
    """加载 baseline：每行一条，跳过 `#` 注释与空行，排序保留重复。"""
    items = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        items.append(line)
    return sorted(items)


def write_baseline(path: Path, entries: list[str]) -> None:
    """全量重写 baseline：头部注释 + 排序条目，UTF-8 + 结尾换行。"""
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# test-naming-guard baseline — 存量豁免清单（自动生成）",
        f"# 键格式: 相对路径:行号: {RULE_LABEL}(函数名)（与脚本违规输出一致）",
        "# 语义: 命中条目豁免；清单外新增违规 fail；已消除条目提示可收窄",
        "# 重写: python3 scripts/test-naming-guard/test_naming_guard.py --update-baseline",
        *sorted(entries),
    ]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def remove_entries(path: Path, entries: list[str]) -> list[str]:
    """行过滤式收窄写回：删除与 entries strip 精确匹配的行，其余原样保留。

    头部注释、空行与既有顺序不变，UTF-8 写回（不重排、不新增行）；返回实际
    删除的条目（去重排序）；文件中不存在给定条目时不报错（no-op）。
    """
    targets = set(entries)
    removed: set[str] = set()
    kept_lines: list[str] = []
    text = path.read_text(encoding="utf-8")
    for raw in text.splitlines():
        line = raw.strip()
        if line in targets:
            removed.add(line)
            continue
        kept_lines.append(raw)
    if not removed:
        return []
    trailing = "\n" if text.endswith("\n") else ""
    path.write_text("\n".join(kept_lines) + trailing, encoding="utf-8")
    return sorted(removed)


# ---------- CLI ----------------------------------------------------------------


def resolve_default_root() -> Path:
    """默认扫描根：仓库根（经 git rev-parse 解析，与 dep-guard 同款）。"""
    return Path(
        subprocess.check_output(
            ["git", "rev-parse", "--show-toplevel"],
            cwd=SCRIPT_DIR,
            text=True,
        ).strip()
    )


@click.command(name="test-naming-guard")
@click.option(
    "--fix",
    "should_fix",
    is_flag=True,
    default=False,
    help="自动收窄 baseline：删除已消除条目并回写；新增违规照常 FAIL，退出码不变。",
)
@click.option(
    "--update-baseline",
    "update_baseline",
    is_flag=True,
    default=False,
    help="以当前全量扫描结果重写 baseline 文件后退出（与 --fix 互斥）。",
)
@click.option(
    "--root",
    type=click.Path(exists=True, file_okay=False, path_type=Path),
    default=None,
    help="扫描根目录（默认仓库根，经 git rev-parse 解析）。",
)
def main(should_fix: bool, update_baseline: bool, root: Path | None) -> None:
    """测试函数 test_ 前缀守卫：baseline=现状，仅对新增违规 FAIL（exit 1）。"""
    if should_fix and update_baseline:
        raise click.UsageError("--update-baseline 与 --fix 互斥（收窄以实际比对为准）")
    scan_target = root if root is not None else resolve_default_root()
    click.echo("[test-naming-guard] test_ 前缀检查：#[test]/#[tokio::test] 函数命名")
    actual = scan_root(scan_target)
    if update_baseline:
        write_baseline(BASELINE_PATH, actual)
        click.echo(
            f"[test-naming-guard] baseline 已全量重写: {BASELINE_PATH}（{len(actual)} 条）"
        )
        click.get_current_context().exit(0)
    baseline_items = load_baseline(BASELINE_PATH) if BASELINE_PATH.exists() else []
    diff = diff_baseline(baseline_items, actual)
    for item in diff.new_items:
        click.echo(f"  [FAIL] 新增违规: {item}")
    for item in diff.eliminated_items:
        click.echo(f"  [info] 可收窄 baseline（违规已消除）: {item}")
    if should_fix and diff.eliminated_items and BASELINE_PATH.exists():
        removed = remove_entries(BASELINE_PATH, diff.eliminated_items)
        for item in removed:
            click.echo(f"  [fix] 已收窄 baseline（违规已消除）: {item}")
    exempt = len(actual) - len(diff.new_items)
    status = "FAIL" if diff.has_new else "PASS"
    click.echo(
        f"[test-naming-guard] 测试函数 {len(actual)} 处（baseline 豁免 {exempt}）/ "
        f"baseline {len(baseline_items)} 条 / 新增 {len(diff.new_items)} / "
        f"已消除 {len(diff.eliminated_items)} → {status}"
    )
    click.get_current_context().exit(1 if diff.has_new else 0)


if __name__ == "__main__":
    main()
