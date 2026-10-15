#!/usr/bin/env python3
"""baseline 引擎：加载 baseline 文件并做三态 diff（新增 / 消除 / 一致）。"""

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
