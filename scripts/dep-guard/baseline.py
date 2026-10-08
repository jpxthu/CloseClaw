#!/usr/bin/env python3
"""baseline 引擎：加载 baseline 文件并做三态 diff（新增 / 消除 / 一致）。"""

from __future__ import annotations

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


def diff_baseline(baseline_items: list[str], actual_items: list[str]) -> BaselineDiff:
    """实际违规 − baseline = 新增（FAIL）；baseline − 实际 = 消除（可收窄）。"""
    base = set(baseline_items)
    actual = set(actual_items)
    return BaselineDiff(
        new_items=sorted(actual - base),
        eliminated_items=sorted(base - actual),
    )
