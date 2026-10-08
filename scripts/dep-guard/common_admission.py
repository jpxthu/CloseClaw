#!/usr/bin/env python3
"""common 准入一致性检查：common 文档条目 ↔ common crate pub 项双向比对。

依据 docs/design/STANDARDS.md「common 准入清单（比对口径）」：

- 清单侧：shared-types.md + core-traits.md 全文中的类型 / trait 条目——各级标题
  与粗体条目（列表项 `- **名称**：…` 或行首 `**名称**：…`，同一行多个粗体段
  逐段计入）；含说明文字的段（固定章节分组标题、语义粗体、双语标题中的中文
  描述段）不构成条目；` / ` 或空白分隔者分别计入；类别修饰词（如 trait）不计；
  类型 / trait 名按 UpperCamelCase 命名规范以大写字母开头，小写标识符段
  （模块名、crate 名等）不构成条目；同名去重。
- 代码侧：common crate 的 pub struct / enum / trait 与 pub type 别名——沿
  lib.rs pub mod 树取公共可达模块内的定义；测试相关模块（`#[cfg(test)]` 声明、
  `mod tests`、inline `mod tests {}`）内的项经公共可达性判定自然排除；机械包装
  别名（`Shared…` = `Arc<…>` 形式）、函数、常量、模块、再导出声明不参与比对。
- 双向比对：清单缺名（common 有 pub 项而清单无条目）∪ 清单多名（清单有条目
  而 common 无 pub 项）= 偏差集合。
"""

from __future__ import annotations

import re
from pathlib import Path

import second_export

COMMON_PKG = second_export.COMMON_PKG
DOC_DIR_RELPATH = ("docs", "design", "common")
DOC_FILES = ("shared-types.md", "core-traits.md")


class CommonAdmissionError(Exception):
    """文档条目解析、mod 树构建或 common crate 定位失败。"""


# ---------------------------------------------------------------------------
# 清单侧：markdown 条目解析
# ---------------------------------------------------------------------------

_CATEGORY_MODIFIERS = frozenset(
    {"trait", "struct", "enum", "union", "type", "alias", "fn", "function",
     "macro", "const", "static", "module", "crate", "mod"}
)
_IDENT_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
_TYPE_NAME_RE = re.compile(r"[A-Z][A-Za-z0-9_]*")
_HEADING_RE = re.compile(r"^\s*#{1,6}\s+(.+?)\s*#*\s*$")
_BOLD_RE = re.compile(r"^\s*(?:[-*+]|\d+\.)?\s*\*\*")
_BOLD_SPAN_RE = re.compile(r"\*\*(.+?)\*\*")
_FENCE_RE = re.compile(r"^\s*(```|~~~)")


def _entry_names(text: str) -> set[str]:
    """一个标题 / 粗体段 → 条目名集合（` / ` 分隔分段，段内全标识符才计）。"""
    names: set[str] = set()
    for part in re.split(r"\s+/\s+", text.strip()):
        tokens = part.split()
        if not tokens:
            continue
        if any(not _IDENT_RE.fullmatch(token) for token in tokens):
            continue
        names.update(
            token
            for token in tokens
            if token not in _CATEGORY_MODIFIERS and _TYPE_NAME_RE.fullmatch(token)
        )
    return names


def parse_doc_entries(doc_dir: Path) -> set[str]:
    """解析 common 文档目录下两份清单文档的标题与粗体条目，返回条目名集合。"""
    names: set[str] = set()
    for filename in DOC_FILES:
        path = doc_dir / filename
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except OSError as exc:
            raise CommonAdmissionError(f"读取 {path} 失败: {exc}") from exc
        in_fence = False
        for line in lines:
            if _FENCE_RE.match(line):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            heading = _HEADING_RE.match(line)
            if heading is not None:
                names |= _entry_names(heading.group(1))
                continue
            if _BOLD_RE.match(line):
                for span in _BOLD_SPAN_RE.finditer(line):
                    names |= _entry_names(span.group(1))
    return names


# ---------------------------------------------------------------------------
# 代码侧：common crate pub 项枚举
# ---------------------------------------------------------------------------

_SHARED_ALIAS_RE = re.compile(r"(?:[A-Za-z_]\w*::)*Arc\s*<")
_TYPE_STMT_RE = re.compile(r"^(?:r#)?([A-Za-z_]\w*)")
_TYPE_RHS_RE = re.compile(r"=\s*(.+)$", re.S)


def _is_shared_wrapper(name: str, stmt_text: str) -> bool:
    """`pub type Shared… = Arc<…>` 机械包装别名判定。"""
    if not name.startswith("Shared"):
        return False
    rhs = _TYPE_RHS_RE.search(stmt_text)
    return bool(rhs) and bool(_SHARED_ALIAS_RE.match(rhs.group(1).strip()))


def _collect_node_names(node: second_export.ModNode, names: set[str]) -> None:
    """单个公共可达 mod 节点 → pub 类型 / trait 名（type 语句 + 定义声明）。"""
    for item in node.items:
        if item.kind != "type" or not item.pub_plain:
            continue
        m = _TYPE_STMT_RE.match(item.text)
        if m and not _is_shared_wrapper(m.group(1), item.text):
            names.add(m.group(1))
    for definition in node.defs:
        if definition.pub_plain and definition.name:
            names.add(definition.name)


def collect_pub_items(crate_root: Path) -> set[str]:
    """枚举 common crate 公共可达的 pub 类型 / trait 名集合。"""
    nodes = second_export.crate_mod_nodes(crate_root)
    if not nodes:
        raise CommonAdmissionError(f"{crate_root}: 未找到 lib 入口（src/lib.rs）")
    names: set[str] = set()
    for node in nodes:
        if node.reachable:
            _collect_node_names(node, names)
    return names


# ---------------------------------------------------------------------------
# 双向比对
# ---------------------------------------------------------------------------


def compare(doc_names: set[str], code_names: set[str]) -> tuple[list[str], list[str]]:
    """返回（清单缺名, 清单多名），各自排序。"""
    return sorted(code_names - doc_names), sorted(doc_names - code_names)


def deviation_items(missing_in_doc: list[str], missing_in_code: list[str]) -> list[str]:
    """偏差集合 → baseline 行格式（排序稳定，便于 review 收窄）。"""
    items = [f"missing-in-doc: {name}" for name in missing_in_doc]
    items += [f"missing-in-code: {name}" for name in missing_in_code]
    return sorted(items)


def find_deviations(repo_root: Path, edge_data) -> list[str]:
    """全量比对：文档条目 ↔ common crate pub 项 → baseline 行格式列表。

    edge_data 为 dep_edges.parse_metadata 的 EdgeData；common crate 定位
    失败时 fail-loud。
    """
    common_dir = second_export.common_crate_dir(edge_data)
    crate_path = edge_data.dir_to_path.get(common_dir)
    if crate_path is None:
        raise CommonAdmissionError(f"common crate 目录缺失路径映射: {common_dir}")
    doc_names = parse_doc_entries(repo_root.joinpath(*DOC_DIR_RELPATH))
    code_names = collect_pub_items(Path(crate_path))
    missing_in_doc, missing_in_code = compare(doc_names, code_names)
    return deviation_items(missing_in_doc, missing_in_code)
