#!/usr/bin/env python3
"""baseline 引擎：加载 baseline 文件、三态 diff（新增 / 消除 / 一致）与收窄写回。

``remove_entries`` 提供行过滤式收窄写回（供 ``check --fix`` 删除已消除
条目并回写），与 load / diff 共同构成 baseline 生命周期闭环。
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path


@dataclass
class BaselineDiff:
    new_items: list[str]
    eliminated_items: list[str]

    @property
    def has_new(self) -> bool:
        return bool(self.new_items)


def load_baseline(path: Path) -> list[str]:
    """加载 baseline：每行一条，跳过 `#` 注释与空行，去重排序。"""
    items = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        items.append(line)
    return sorted(set(items))


def remove_entries(path: Path, entries: list[str]) -> list[str]:
    """行过滤式收窄写回：删除与 entries strip 精确匹配的行，其余原样保留。

    逐行读取 baseline，strip 后与给定条目相等的行删除，头部注释、空行与
    其余条目的既有顺序不变，UTF-8 写回（不重排、不新增行）；返回实际删除
    的条目（去重排序）；文件中不存在给定条目时不报错（no-op，不触碰文件）。
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


def diff_baseline(
    baseline_items: list[str],
    actual_items: list[str],
    key: Callable[[str], str] | None = None,
) -> BaselineDiff:
    """实际违规 − baseline = 新增（FAIL）；baseline − 实际 = 消除（可收窄）。

    key 提供时（仿 sorted 的 key 惯例），新增 / 消除判定以 key(entry) 为比较
    键，报告仍返回两侧原始条目（FAIL 报文保留行号等定位信息）；key=None 时
    与整条目字符串集合差完全一致。同 key 多条目并存时保留后出现者（dict
    覆盖语义），消除侧仅报该条；两侧均按去重排序输入构建，属 no-op 说明。
    """
    keyfunc = (lambda item: item) if key is None else key
    base = {keyfunc(item): item for item in baseline_items}
    actual = {keyfunc(item): item for item in actual_items}
    return BaselineDiff(
        new_items=sorted(actual[k] for k in actual.keys() - base.keys()),
        eliminated_items=sorted(base[k] for k in base.keys() - actual.keys()),
    )
