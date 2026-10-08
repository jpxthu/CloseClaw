#!/usr/bin/env python3
"""解析 docs/design/STANDARDS.md「依赖方向允许边表」。

输出 {crate 目录名: 允许依赖目录名集合}；组合根（daemon + 根 crate）为
ALLOW_ALL 哨兵；未列入本表的 crate 由调用方套用 DEFAULT_DOMAIN_ALLOWED
（领域层规则）。解析失败一律抛 AllowTableError（fail-loud）。
"""

from __future__ import annotations

import re
from pathlib import Path

DEFAULT_DOMAIN_ALLOWED = frozenset({"common", "platform", "debug_log"})
ALLOW_ALL = "<all>"
ROOT_CRATE = "<root-crate>"

_SECTION_HEADING = re.compile(r"^#{1,6}\s.*依赖方向允许边表")
_TABLE_SEPARATOR = re.compile(r"^\s*\|[-: |]*-[-: |]*\|\s*$")
_BACKTICK = re.compile(r"`([^`]+)`")
_ROOT_MARKER = re.compile(r"根\s*crate\s*`([^`]+)`")


class AllowTableError(Exception):
    """允许边表缺失或格式破坏。"""


def load_allow_table(standards_path: Path) -> dict:
    """读取 STANDARDS.md 并解析允许边表。"""
    text = standards_path.read_text(encoding="utf-8")
    return parse_allow_table(text)


def parse_allow_table(text: str) -> dict:
    """解析允许边表 markdown 表格 → {crate 目录名: 允许集合 | ALLOW_ALL}。"""
    lines = text.splitlines()
    start = _find_section(lines)
    rows = _extract_table_rows(lines, start)
    table: dict = {}
    for row in rows:
        cells = _split_row(row)
        if len(cells) != 3:
            raise AllowTableError(f"允许边表行格式破坏（应为 3 列）: {row}")
        layer, crate_cell, allowed_cell = (cell.strip() for cell in cells)
        if not layer:
            raise AllowTableError(f"允许边表行「层」列为空: {row}")
        allowed = _allowed_set(allowed_cell, row)
        for name in _crate_keys(crate_cell, row):
            table[name] = allowed
    if not table:
        raise AllowTableError("允许边表无数据行")
    return table


def _find_section(lines: list[str]) -> int:
    for index, line in enumerate(lines):
        if _SECTION_HEADING.match(line):
            return index
    raise AllowTableError("STANDARDS.md 中未找到「依赖方向允许边表」章节")


def _extract_table_rows(lines: list[str], start: int) -> list[str]:
    header_index = None
    for index in range(start, len(lines)):
        if lines[index].lstrip().startswith("|"):
            header_index = index
            break
    if header_index is None or header_index + 1 >= len(lines):
        raise AllowTableError("「依赖方向允许边表」章节下未找到表格")
    if not _TABLE_SEPARATOR.match(lines[header_index + 1]):
        raise AllowTableError(f"允许边表表头分隔行缺失或格式破坏: {lines[header_index + 1]}")
    if len(_split_row(lines[header_index])) != 3:
        raise AllowTableError(f"允许边表表头应为 3 列: {lines[header_index]}")
    rows = []
    for line in lines[header_index + 2:]:
        if not line.lstrip().startswith("|"):
            break
        rows.append(line)
    return rows


def _split_row(line: str) -> list[str]:
    return line.strip().removeprefix("|").removesuffix("|").split("|")


def _crate_keys(cell: str, row: str) -> list[str]:
    root_names = _ROOT_MARKER.findall(cell)
    names = [name for name in _BACKTICK.findall(cell) if name not in root_names]
    keys = [ROOT_CRATE for _ in root_names] + names
    keys = list(dict.fromkeys(keys))
    if not keys:
        raise AllowTableError(f"允许边表行 crate 列无 crate 名: {row}")
    return keys


def _allowed_set(cell: str, row: str) -> frozenset[str] | str:
    if cell.startswith("全部"):
        return ALLOW_ALL
    if cell.startswith("无"):
        return frozenset()
    names = _BACKTICK.findall(cell)
    if not names:
        raise AllowTableError(f"允许边表行允许依赖列无法解析: {row}")
    return frozenset(names)
