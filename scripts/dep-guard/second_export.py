#!/usr/bin/env python3
"""禁止二次出口检查：common 之外的 crate 不得把 common 项经自己的公共路径再暴露。

依据 docs/design/STANDARDS.md「禁止二次出口」：对 `closeclaw_common` 之外的每个
crate，扫描 `pub use`（含跨行语句、通配 `*`、`as` 别名、use 树）、`pub type <名称>
= <common 项>` 与 `pub extern crate` 中源为 `closeclaw_common`（或 crate 内
`crate::common` shim）的声明；所在 mod 自 crate 根经全部 `pub` 项链公共可达者
构成二次出口。

可达性构建（自 lib 入口起——Cargo.toml `[lib] path` 优先，缺省 src/lib.rs；
main.rs bin 目标不构成公共 API）：
- 沿 `pub mod` / `mod` 声明递归建 mod 树，处理 `#[path]`、`#[cfg(test)]`
  （cfg(test) 子树不可达）、inline `mod X { }` 块（括号深度状态机）；
- 私有 mod 被 `pub use` 再导出（命名或 `::*` 通配，含 `self::` / `crate::` /
  `super::` 形式）时，近似视为公共可达，迭代至不动点；
- 再导出本 crate 自身定义项（`crate::` / `self::` 源，`crate::common` shim 除外）
  不算二次出口。

已知近似与盲区（未报警不代表合规，兜底靠 review）：
- 经 `use closeclaw_common as …` 别名后的再导出不识别；
- 私有 mod 内 common 项被逐名再导出（`pub use hidden::Foo;`）不触发可达性
  传播（仅跟踪 mod 本体被再导出的情形）；
- fn 体内语句归入宿主 mod 块（rustc 不容 fn 体内 `pub use`，此处仅记录
  解析近似，勿依赖）；
- 宏展开生成的声明（`macro_rules!` 产出及其调用）不扫描；
- 仅识别 `closeclaw-common` 包名为 common；common 子 crate 若有则不豁免；
- （common-admission 清单侧）标题混排 / 反引号包裹的条目名不识别。
"""

from __future__ import annotations

import re
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

COMMON_PKG = "closeclaw-common"
SOURCE_CRATE = "closeclaw_common"
SOURCE_SHIM_PREFIX = "crate::common"


class SecondExportError(Exception):
    """mod 树构建或源码扫描失败。"""


@dataclass
class RawItem:
    """文件内一条 use / type / extern crate 声明。"""

    kind: str  # "use" | "type" | "extern"
    text: str
    line: int
    pub_plain: bool  # 可见性为裸 `pub` 且非 #[cfg(test)]
    cfg_test: bool


@dataclass
class RawFileMod:
    """文件内一条 `mod X;` 文件模块声明。"""

    name: str
    line: int
    pub_plain: bool  # 裸 `pub` 且非 #[cfg(test)]
    path_attr: str | None  # #[path = "..."]


@dataclass
class RawDef:
    """文件内一条类型 / trait 定义声明（struct / enum / union / trait）。"""

    kind: str
    name: str
    line: int
    pub_plain: bool  # 裸 `pub` 且非 #[cfg(test)]
    cfg_test: bool


@dataclass
class ModBlock:
    """一次文件扫描得到的模块块（文件根或 inline mod）。"""

    name: str
    is_pub: bool
    items: list[RawItem] = field(default_factory=list)
    defs: list[RawDef] = field(default_factory=list)
    file_mods: list[RawFileMod] = field(default_factory=list)
    children: list["ModBlock"] = field(default_factory=list)


@dataclass
class ModNode:
    """crate mod 树节点。"""

    name: str
    file_rel: str  # 所在文件相对 crate 目录（posix，inline mod 沿用宿主文件）
    declared_pub: bool
    module_path: tuple[str, ...]
    parent: "ModNode | None"
    inline: bool = False
    children: dict[str, "ModNode"] = field(default_factory=dict)
    items: list[RawItem] = field(default_factory=list)
    defs: list[RawDef] = field(default_factory=list)
    reachable: bool = False


# ---------------------------------------------------------------------------
# 源码扫描（括号深度状态机）
# ---------------------------------------------------------------------------


class _ItemScanner:
    """按字符扫描 Rust 源文件，提取模块声明与 use / type / extern crate 语句。

    跳过注释、字符串 / 字符字面量（含 raw / byte 前缀）；用括号深度跟踪
    inline mod 块边界，排除 `mod tests {}` 等块内语句对后续解析的影响。
    """

    _ITEM_KEYWORDS = frozenset({
        "fn", "struct", "enum", "union", "trait", "impl",
        "const", "static", "macro", "macro_rules",
    })
    _STRING_PREFIX = re.compile(r"(?:r|br)(#*)|b|c|cr")

    def __init__(self, text: str) -> None:
        self.text = text
        self.n = len(text)
        self.i = 0
        self.line = 1
        self.depth = 0
        self.attrs: list[str] = []
        self.pub_plain = False
        root = ModBlock(name="", is_pub=True)
        self.block_stack: list[ModBlock] = [root]
        self.open_depths: list[int] = [0]

    # ---- 基础原语 ----

    def _peek(self, offset: int = 0) -> str:
        j = self.i + offset
        return self.text[j] if j < self.n else ""

    def _skip_trivia(self) -> None:
        while self.i < self.n:
            ch = self.text[self.i]
            if ch in " \t\r\n":
                if ch == "\n":
                    self.line += 1
                self.i += 1
            elif ch == "/" and self._peek(1) == "/":
                while self.i < self.n and self.text[self.i] != "\n":
                    self.i += 1
            elif ch == "/" and self._peek(1) == "*":
                self._skip_block_comment()
            else:
                return

    def _skip_block_comment(self) -> None:
        self.i += 2
        level = 1
        while self.i < self.n and level:
            if self.text.startswith("/*", self.i):
                level += 1
                self.i += 2
            elif self.text.startswith("*/", self.i):
                level -= 1
                self.i += 2
            else:
                if self.text[self.i] == "\n":
                    self.line += 1
                self.i += 1

    def _skip_string(self, hashes: int) -> None:
        self.i += 1  # 越过开引号
        terminator = '"' + "#" * hashes
        while self.i < self.n:
            if hashes == 0 and self.text[self.i] == "\\":
                if self._peek(1) == "\n":
                    self.line += 1
                self.i += 2
                continue
            if self.text.startswith(terminator, self.i):
                self.i += len(terminator)
                return
            if self.text[self.i] == "\n":
                self.line += 1
            self.i += 1

    def _skip_char_or_lifetime(self) -> None:
        if self._peek(1) == "\\":
            j = self.i + 2
            if j < self.n and self.text[j] == "\n":
                self.line += 1
            k = self.text.find("'", j + 1)
            self.i = k + 1 if k != -1 else j + 1
            return
        if self._peek(2) == "'":
            self.i += 3
            return
        self.i += 1  # 生命周期 'a

    def _read_ident(self) -> str:
        start = self.i
        while self.i < self.n and (self.text[self.i].isalnum() or self.text[self.i] == "_"):
            self.i += 1
        return self.text[start:self.i]

    def _read_attribute(self) -> None:
        self.i += 1  # '#'
        if self._peek() == "!":
            self.i += 1
        self._skip_trivia()
        if self._peek() != "[":
            return
        self.i += 1
        buf_start = self.i
        level = 1
        while self.i < self.n and level:
            ch = self.text[self.i]
            if ch == "/" and self._peek(1) == "/":
                while self.i < self.n and self.text[self.i] != "\n":
                    self.i += 1
            elif ch == "/" and self._peek(1) == "*":
                self._skip_block_comment()
            elif ch == '"':
                self._skip_string(0)
            elif ch == "'":
                self._skip_char_or_lifetime()
            elif ch == "[":
                level += 1
                self.i += 1
            elif ch == "]":
                level -= 1
                self.i += 1
            else:
                if ch == "\n":
                    self.line += 1
                self.i += 1
        self.attrs.append(self.text[buf_start:self.i - 1].strip())

    def _collect_statement(self) -> str:
        """自当前位置收集到括号嵌套归零的 `;`（含 use 树花括号）。"""
        start = self.i
        paren = bracket = brace = 0
        while self.i < self.n:
            ch = self.text[self.i]
            if ch == "/" and self._peek(1) == "/":
                while self.i < self.n and self.text[self.i] != "\n":
                    self.i += 1
            elif ch == "/" and self._peek(1) == "*":
                self._skip_block_comment()
            elif ch == '"':
                self._skip_string(0)
            elif ch == "'":
                self._skip_char_or_lifetime()
            elif ch == "(":
                paren += 1
                self.i += 1
            elif ch == ")":
                paren -= 1
                self.i += 1
            elif ch == "[":
                bracket += 1
                self.i += 1
            elif ch == "]":
                bracket -= 1
                self.i += 1
            elif ch == "{":
                brace += 1
                self.i += 1
            elif ch == "}":
                brace -= 1
                self.i += 1
            elif ch == ";" and paren == 0 and bracket == 0 and brace == 0:
                self.i += 1
                return self.text[start:self.i - 1].strip()
            else:
                if ch == "\n":
                    self.line += 1
                self.i += 1
        return self.text[start:self.i].strip()

    # ---- 属性语义 ----

    def _cfg_test(self) -> bool:
        return any(re.search(r"\bcfg\s*\(\s*test\b", a) for a in self.attrs)

    def _path_attr(self) -> str | None:
        for a in self.attrs:
            m = re.search(r"\bpath\s*=\s*(?:r#*)?\"([^\"]+)\"", a)
            if m:
                return m.group(1)
        return None

    def _reset_pending(self) -> None:
        self.attrs = []
        self.pub_plain = False

    # ---- 主循环 ----

    def run(self) -> ModBlock:
        while self.i < self.n:
            self._skip_trivia()
            if self.i >= self.n:
                break
            ch = self.text[self.i]
            if ch == "#":
                self._read_attribute()
                continue
            if ch == '"':
                self._skip_string(0)
                self._reset_pending()
                continue
            if ch == "'":
                self._skip_char_or_lifetime()
                continue
            if ch == "{":
                self.depth += 1
                self.i += 1
                self._reset_pending()
                continue
            if ch == "}":
                self.depth -= 1
                self.i += 1
                while len(self.open_depths) > 1 and self.open_depths[-1] > self.depth:
                    self.open_depths.pop()
                    self.block_stack.pop()
                self._reset_pending()
                continue
            if ch == ";":
                self.i += 1
                self._reset_pending()
                continue
            if ch.isdigit():
                while self.i < self.n and (self.text[self.i].isalnum() or self.text[self.i] == "_"):
                    self.i += 1
                continue
            if not (ch.isalpha() or ch == "_"):
                self.i += 1
                continue
            word = self._read_ident()
            if self._peek() == '"' and self._STRING_PREFIX.fullmatch(word):
                m = self._STRING_PREFIX.fullmatch(word)
                self._skip_string(len(m.group(1) or "") if m else 0)
                self._reset_pending()
                continue
            self._dispatch(word)
        return self.block_stack[0]

    def _dispatch(self, word: str) -> None:
        current = self.block_stack[-1]
        if word == "pub":
            self.pub_plain = True
            save = self.i
            self._skip_trivia()
            if self._peek() == "(":
                j = self.text.find(")", self.i)
                inner = self.text[self.i + 1: j if j != -1 else self.i + 1].strip()
                head = inner.split("(")[0].strip()
                if head in ("crate", "super") or head.startswith("in"):
                    self.pub_plain = False
                self.i = j + 1 if j != -1 else self.i
            else:
                self.i = save
            return
        if word == "mod":
            stmt_line = self.line
            self._skip_trivia()
            nxt = self._peek()
            if not (nxt.isalpha() or nxt == "_"):
                self._reset_pending()
                return
            name = self._read_ident()
            self._skip_trivia()
            nxt = self._peek()
            is_pub = self.pub_plain and not self._cfg_test()
            if nxt == "{":
                self.i += 1
                self.depth += 1
                block = ModBlock(name=name, is_pub=is_pub)
                current.children.append(block)
                self.block_stack.append(block)
                self.open_depths.append(self.depth)
                self._reset_pending()
                return
            if nxt == ";":
                self.i += 1
                current.file_mods.append(
                    RawFileMod(
                        name=name, line=stmt_line, pub_plain=is_pub, path_attr=self._path_attr()
                    )
                )
                self._reset_pending()
                return
            self._reset_pending()
            return
        if word in ("use", "type"):
            stmt_line = self.line
            body = self._collect_statement()
            current.items.append(
                RawItem(kind=word, text=body, line=stmt_line,
                        pub_plain=self.pub_plain and not self._cfg_test(),
                        cfg_test=self._cfg_test())
            )
            self._reset_pending()
            return
        if word == "extern":
            save = self.i
            self._skip_trivia()
            nxt = self._peek()
            if nxt.isalpha() or nxt == "_":
                inner = self._read_ident()
                if inner == "crate":
                    stmt_line = self.line
                    body = self._collect_statement()
                    current.items.append(
                        RawItem(kind="extern", text=body, line=stmt_line,
                                pub_plain=self.pub_plain and not self._cfg_test(),
                                cfg_test=self._cfg_test())
                    )
                    self._reset_pending()
                    return
            self.i = save
            return
        if word in ("struct", "enum", "union", "trait"):
            stmt_line = self.line
            self._skip_trivia()
            nxt = self._peek()
            name = self._read_ident() if (nxt.isalpha() or nxt == "_") else ""
            current.defs.append(
                RawDef(
                    kind=word,
                    name=name,
                    line=stmt_line,
                    pub_plain=self.pub_plain and not self._cfg_test(),
                    cfg_test=self._cfg_test(),
                )
            )
            self._reset_pending()
            return
        if word in self._ITEM_KEYWORDS:
            self._reset_pending()


def scan_source(text: str) -> ModBlock:
    """扫描一份 Rust 源文件文本，返回文件根模块块。"""
    return _ItemScanner(text).run()


# ---------------------------------------------------------------------------
# use 树解析
# ---------------------------------------------------------------------------


class _UseTreeParser:
    """把 use 语句体解析为叶子路径列表（含通配与 as 别名）。"""

    def __init__(self, text: str) -> None:
        self.s = text
        self.n = len(text)
        self.i = 0

    def _peek(self, offset: int = 0) -> str:
        j = self.i + offset
        return self.s[j] if j < self.n else ""

    def _skip_ws(self) -> None:
        while self.i < self.n:
            ch = self.s[self.i]
            if ch in " \t\r\n":
                self.i += 1
            elif ch == "/" and self._peek(1) == "/":
                while self.i < self.n and self.s[self.i] != "\n":
                    self.i += 1
            elif ch == "/" and self._peek(1) == "*":
                end = self.s.find("*/", self.i + 2)
                self.i = end + 2 if end != -1 else self.n
            else:
                return

    def _read_ident(self) -> str:
        self._skip_ws()
        j = self.i
        if self.s.startswith("r#", j):
            j += 2
        if j < self.n and (self.s[j].isalpha() or self.s[j] == "_"):
            j += 1
            while j < self.n and (self.s[j].isalnum() or self.s[j] == "_"):
                j += 1
            ident = self.s[self.i:j]
            self.i = j
            return ident
        return ""

    def _try_keyword(self, word: str) -> bool:
        save = self.i
        self._skip_ws()
        if self.s.startswith(word, self.i):
            after = self.i + len(word)
            if after >= self.n or not (self.s[after].isalnum() or self.s[after] == "_"):
                self.i = after
                return True
        self.i = save
        return False

    def parse(self) -> list[tuple[str, str | None]]:
        self._skip_ws()
        if self._peek() == ":" and self._peek(1) == ":":
            self.i += 2
        leaves = self._parse_group([])
        return [leaf for leaf in leaves if leaf[0]]

    def _parse_group(self, prefix: list[str]) -> list[tuple[str, str | None]]:
        leaves: list[tuple[str, str | None]] = []
        while True:
            self._skip_ws()
            if self._peek() in ("", "}", ","):
                break
            leaves.extend(self._parse_leaf(prefix))
            self._skip_ws()
            if self._peek() == ",":
                self.i += 1
                continue
            break
        return leaves

    def _parse_leaf(self, prefix: list[str]) -> list[tuple[str, str | None]]:
        segs = list(prefix)
        while True:
            self._skip_ws()
            ch = self._peek()
            if ch == "{":
                self.i += 1
                inner = self._parse_group(segs)
                self._skip_ws()
                if self._peek() == "}":
                    self.i += 1
                return inner
            if ch == "*":
                self.i += 1
                return [("::".join(segs + ["*"]), None)]
            if ch == ":":
                self.i += 2 if self._peek(1) == ":" else 1
                continue
            ident = self._read_ident()
            if not ident:
                self.i += 1
                continue
            self._skip_ws()
            if self._peek() == ":":
                segs.append(ident)
                continue
            alias = None
            if self._try_keyword("as"):
                alias = self._read_ident() or None
            return [("::".join(segs + [ident]), alias)]


def parse_use_leaves(body: str) -> list[tuple[str, str | None]]:
    """`use` 语句体 → [(完整路径, as 别名或 None)]。"""
    return _UseTreeParser(body).parse()


# ---------------------------------------------------------------------------
# mod 树构建与可达性
# ---------------------------------------------------------------------------

MODRS_STEMS = frozenset({"mod", "lib", "main"})


def resolve_mod_file(base_dir: Path, fm: RawFileMod) -> Path | None:
    if fm.path_attr:
        candidate = base_dir / fm.path_attr
        if candidate.is_file():
            return candidate
        candidate = base_dir / fm.path_attr / "mod.rs"
        return candidate if candidate.is_file() else None
    candidate = base_dir / f"{fm.name}.rs"
    if candidate.is_file():
        return candidate
    candidate = base_dir / fm.name / "mod.rs"
    return candidate if candidate.is_file() else None


def _attach_inline(
    node: ModNode,
    block: ModBlock,
    child_base: Path,
    crate_root: Path,
    visited: set[str],
    nodes: list[ModNode],
) -> None:
    node.items = list(block.items)
    node.defs = list(block.defs)
    for sub in block.children:
        child = ModNode(
            name=sub.name,
            file_rel=node.file_rel,
            declared_pub=sub.is_pub,
            module_path=node.module_path + (sub.name,),
            parent=node,
            inline=True,
        )
        node.children[sub.name] = child
        nodes.append(child)
        _attach_inline(child, sub, child_base / sub.name, crate_root, visited, nodes)
    for fm in block.file_mods:
        _add_file_mod(node, child_base, fm, crate_root, visited, nodes)


def _add_file_mod(
    parent: ModNode,
    base_dir: Path,
    fm: RawFileMod,
    crate_root: Path,
    visited: set[str],
    nodes: list[ModNode],
) -> None:
    target = resolve_mod_file(base_dir, fm)
    if target is None or str(target) in visited:
        return
    visited.add(str(target))
    try:
        rel = target.resolve().relative_to(crate_root.resolve()).as_posix()
    except ValueError:
        rel = target.as_posix()
    _scan_file(
        target,
        rel=rel,
        name=fm.name,
        declared_pub=fm.pub_plain,
        module_path=parent.module_path + (fm.name,),
        parent=parent,
        crate_root=crate_root,
        visited=visited,
        nodes=nodes,
    )


def _scan_file(
    file_path: Path,
    *,
    rel: str,
    name: str,
    declared_pub: bool,
    module_path: tuple[str, ...],
    parent: ModNode | None,
    crate_root: Path,
    visited: set[str],
    nodes: list[ModNode],
) -> ModNode:
    try:
        text = file_path.read_text(encoding="utf-8")
    except OSError as exc:
        raise SecondExportError(f"读取 {file_path} 失败: {exc}") from exc
    block = scan_source(text)
    node = ModNode(
        name=name,
        file_rel=rel,
        declared_pub=declared_pub,
        module_path=module_path,
        parent=parent,
    )
    node.items = list(block.items)
    node.defs = list(block.defs)
    nodes.append(node)
    if parent is not None:
        parent.children.setdefault(name, node)

    host_dir = file_path.parent
    stem = file_path.stem
    child_base = host_dir / stem if stem not in MODRS_STEMS else host_dir
    for sub in block.children:
        child = ModNode(
            name=sub.name,
            file_rel=rel,
            declared_pub=sub.is_pub,
            module_path=module_path + (sub.name,),
            parent=node,
            inline=True,
        )
        node.children.setdefault(sub.name, child)
        nodes.append(child)
        _attach_inline(child, sub, child_base / sub.name, crate_root, visited, nodes)
    for fm in block.file_mods:
        _add_file_mod(node, child_base, fm, crate_root, visited, nodes)
    return node


def _match_reexport_target(node: ModNode, path: str) -> ModNode | None:
    """`pub use <path>` 是否再导出 node 的（或兄弟）子模块 → 目标节点。"""
    base = path[:-3] if path.endswith("::*") else path
    if base.startswith("::"):
        base = base[2:]
    segs = base.split("::") if base else []
    if len(segs) == 1:
        return node.children.get(segs[0])
    if len(segs) == 2 and segs[0] == "self":
        return node.children.get(segs[1])
    if len(segs) == 2 and segs[0] == "super" and node.parent is not None:
        return node.parent.children.get(segs[1])
    if segs and segs[0] == "crate":
        expected = ("crate",) + node.module_path
        if len(segs) == len(expected) + 1 and tuple(segs[:-1]) == expected:
            return node.children.get(segs[-1])
    return None


def _propagate(nodes: list[ModNode]) -> None:
    """可达性不动点：pub 项链 + 私有 mod 被 pub use 再导出的近似（单调置位）。"""
    changed = True
    while changed:
        changed = False
        for node in nodes:
            if node.reachable:
                continue
            want = node.parent is None or (node.parent.reachable and node.declared_pub)
            if want:
                node.reachable = True
                changed = True
        for node in nodes:
            if not node.reachable:
                continue
            for item in node.items:
                if item.kind != "use" or not item.pub_plain:
                    continue
                for path, _alias in parse_use_leaves(item.text):
                    target = _match_reexport_target(node, path)
                    if target is not None and not target.reachable:
                        target.reachable = True
                        changed = True


# ---------------------------------------------------------------------------
# 源分类与汇总
# ---------------------------------------------------------------------------

_TYPE_RHS = re.compile(r"=\s*(.+)$", re.S)
_LEADING_PATH = re.compile(r"^(?:r#)?[A-Za-z_]\w*(?:::(?:r#)?[A-Za-z_]\w*)*")


def _is_common_source(path: str) -> bool:
    if path.startswith("::"):
        path = path[2:]
    if not path:
        return False
    head = path.split("::", 1)[0]
    if head == SOURCE_CRATE:
        return True
    return path == SOURCE_SHIM_PREFIX or path.startswith(SOURCE_SHIM_PREFIX + "::")


def _normalize(path: str) -> str:
    return path[2:] if path.startswith("::") else path


def _item_source_leaves(item: RawItem) -> list[tuple[str, str | None]]:
    """语句中源为 common 的叶子路径（附 as 别名）。"""
    if item.kind == "use":
        return [
            (_normalize(path), alias)
            for path, alias in parse_use_leaves(item.text)
            if _is_common_source(path)
        ]
    if item.kind == "extern":
        m = re.match(r"^(?:r#)?([A-Za-z_]\w*)", item.text.strip())
        if m and m.group(1) == SOURCE_CRATE:
            return [(SOURCE_CRATE, None)]
        return []
    if item.kind == "type":
        m = _TYPE_RHS.search(item.text)
        if not m:
            return []
        path = _normalize(m.group(1).strip())
        pm = _LEADING_PATH.match(path)
        if pm and _is_common_source(pm.group(0)):
            return [(pm.group(0), None)]
        return []
    return []


def lib_entry(crate_root: Path) -> Path | None:
    """crate lib 入口文件：Cargo.toml `[lib] path` 优先，缺省 src/lib.rs。"""
    manifest = crate_root / "Cargo.toml"
    try:
        with manifest.open("rb") as fh:
            data = tomllib.load(fh)
    except FileNotFoundError:
        pass
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise SecondExportError(f"解析 {manifest} 失败: {exc}") from exc
    else:
        lib = data.get("lib")
        if isinstance(lib, dict) and isinstance(lib.get("path"), str):
            candidate = crate_root / lib["path"]
            return candidate if candidate.is_file() else None
    candidate = crate_root / "src" / "lib.rs"
    if candidate.is_file():
        return candidate
    candidate = crate_root / "src" / "mod.rs"
    return candidate if candidate.is_file() else None


def crate_mod_nodes(crate_root: Path) -> list[ModNode]:
    """构建 crate mod 树并完成公共可达性传播，返回全部节点。

    lib 入口缺失时返回空表；cfg(test) 子树不可达，私有 mod 经 `pub use`
    再导出的近似可达性与二次出口检查共享同一实现。
    """
    entry = lib_entry(crate_root)
    if entry is None:
        return []
    nodes: list[ModNode] = []
    visited: set[str] = set()
    root = _scan_file(
        entry,
        rel=entry.relative_to(crate_root).as_posix(),
        name="",
        declared_pub=True,
        module_path=(),
        parent=None,
        crate_root=crate_root,
        visited=visited,
        nodes=nodes,
    )
    root.reachable = True
    _propagate(nodes)
    return nodes


def crate_second_exports(crate_root: Path, pkg: str) -> list[str]:
    """扫描单个 crate（crate 根目录）→ baseline 行格式列表。"""
    nodes = crate_mod_nodes(crate_root)
    if not nodes:
        return []
    found: set[str] = set()
    for node in nodes:
        if not node.reachable:
            continue
        for item in node.items:
            if not item.pub_plain:
                continue
            for path, alias in _item_source_leaves(item):
                entry = f"{pkg}: {node.file_rel}:{item.line} {path}"
                if alias:
                    entry += f" as {alias}"
                found.add(entry)
    return sorted(found)


def common_crate_dir(edge_data) -> str:
    """在 workspace 中定位 common crate 目录；找不到时 fail-loud。"""
    for directory, pkg in edge_data.dir_to_pkg.items():
        if pkg == COMMON_PKG:
            return directory
    raise SecondExportError(f"workspace 中未找到 {COMMON_PKG} crate")


def find_second_exports(edge_data) -> list[str]:
    """对 common 之外的每个 workspace crate 扫描，汇总 baseline 行格式列表。

    edge_data 为 dep_edges.parse_metadata 的 EdgeData（用于 crate 目录定位
    与包名映射）；找不到 common crate 时 fail-loud。
    """
    common_dir = common_crate_dir(edge_data)

    found: set[str] = set()
    for directory, crate_path in edge_data.dir_to_path.items():
        if directory == common_dir:
            continue
        pkg = edge_data.dir_to_pkg.get(directory, directory)
        found.update(crate_second_exports(Path(crate_path), pkg))
    return sorted(found)
