#!/usr/bin/env python3
"""common_admission 单元测试（条目提取、pub 项枚举、双向比对）。"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import common_admission
import dep_edges
import second_export


def _parse(text: str) -> set[str]:
    """把一段 markdown 作为 shared-types.md 解析（写入临时文档目录）。"""
    with tempfile.TemporaryDirectory() as tmp:
        doc_dir = Path(tmp)
        (doc_dir / "shared-types.md").write_text(text, encoding="utf-8")
        (doc_dir / "core-traits.md").write_text("", encoding="utf-8")
        return common_admission.parse_doc_entries(doc_dir)


def _make_crate(case: unittest.TestCase, files: dict[str, str]) -> Path:
    """在临时目录构造 crate 树（key 为相对 crate 目录路径），返回 crate 目录。

    临时目录经 case.addCleanup 挂接清理，测试后无残留。
    """
    tmp = tempfile.TemporaryDirectory()
    case.addCleanup(tmp.cleanup)
    root = Path(tmp.name)
    for rel, content in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    return root


class EntryNamesTest(unittest.TestCase):
    def test_slash_separated_names(self) -> None:
        self.assertEqual(
            common_admission._entry_names("UnifiedResponse / UnifiedUsage"),
            {"UnifiedResponse", "UnifiedUsage"},
        )

    def test_modifier_word_dropped(self) -> None:
        self.assertEqual(common_admission._entry_names("Tool trait"), {"Tool"})
        self.assertEqual(common_admission._entry_names("trait"), set())

    def test_chinese_part_rejected(self) -> None:
        self.assertEqual(common_admission._entry_names("内容段落解析"), set())
        self.assertEqual(
            common_admission._entry_names("ContentSegment / 内容段落解析"),
            {"ContentSegment"},
        )

    def test_group_title_with_identifier_word_rejected(self) -> None:
        self.assertEqual(common_admission._entry_names("Agent 能力查询"), set())
        self.assertEqual(common_admission._entry_names("LLM 调用与流式渲染"), set())

    def test_lowercase_identifier_not_type_name(self) -> None:
        self.assertEqual(common_admission._entry_names("tools"), set())


class DocEntriesTest(unittest.TestCase):
    def test_headings_all_levels(self) -> None:
        text = "# 共享类型\n## 概述\n### Foo\n#### Tool trait\n### Bar / Baz\n"
        self.assertEqual(_parse(text), {"Foo", "Tool", "Bar", "Baz"})

    def test_bold_list_item_and_line_start(self) -> None:
        text = (
            "- **Foo**：说明\n"
            "**Bar**——说明\n"
            "**Baz** 字段：\n"
            "1. **Qux**：说明\n"
            "正文引用 **NotEntry** 不计\n"
        )
        self.assertEqual(_parse(text), {"Foo", "Bar", "Baz", "Qux"})

    def test_bold_line_multi_span(self) -> None:
        text = "- **Foo** / **Bar**：说明；**Baz** 为相关载荷。\n"
        self.assertEqual(_parse(text), {"Foo", "Bar", "Baz"})

    def test_group_titles_excluded(self) -> None:
        text = "## 概述\n## 架构\n## 数据流\n## 模块关系\n### 消息/内容辅助类型\n### 工具契约载荷族\n"
        self.assertEqual(_parse(text), set())

    def test_semantic_bold_excluded(self) -> None:
        text = (
            "**文本类变体**：\n"
            "**DslInstruction 结构**：\n"
            "- **Text 是唯一可能包含 DSL 指令的变体**。说明\n"
            "> **本文档是权威清单。** 说明\n"
            "| **Text** | 表格内粗体不计 |\n"
        )
        self.assertEqual(_parse(text), set())

    def test_module_name_bold_excluded(self) -> None:
        text = "- **tools**（实现 Tool、ToolRegistrar）\n"
        self.assertEqual(_parse(text), set())

    def test_dedup_across_heading_and_bold(self) -> None:
        text = "### Foo\n- **Foo**：说明\n"
        self.assertEqual(_parse(text), {"Foo"})

    def test_code_fence_skipped(self) -> None:
        text = "```\n### NotEntry\n- **AlsoNot**：x\n```\n### RealEntry\n"
        self.assertEqual(_parse(text), {"RealEntry"})

    def test_missing_doc_file_fails_loud(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(common_admission.CommonAdmissionError):
                common_admission.parse_doc_entries(Path(tmp))


class CollectPubItemsTest(unittest.TestCase):
    def test_pub_defs_counted(self) -> None:
        crate = _make_crate(self, {
            "src/lib.rs": "pub mod a;\n",
            "src/a.rs": "pub struct S;\npub enum E { X }\npub trait T {}\n",
        })
        self.assertEqual(common_admission.collect_pub_items(crate), {"S", "E", "T"})

    def test_pub_union_excluded(self) -> None:
        crate = _make_crate(self, {
            "src/lib.rs": "pub mod a;\n",
            "src/a.rs": "pub union U { x: u8 }\npub struct S;\n",
        })
        self.assertEqual(common_admission.collect_pub_items(crate), {"S"})

    def test_private_and_restricted_defs_excluded(self) -> None:
        crate = _make_crate(self, {
            "src/lib.rs": "pub mod a;\n",
            "src/a.rs": "struct P;\npub(crate) struct Q;\npub(super) struct R;\n",
        })
        self.assertEqual(common_admission.collect_pub_items(crate), set())

    def test_cfg_test_mod_excluded(self) -> None:
        crate = _make_crate(self, {
            "src/lib.rs": "pub mod a;\n#[cfg(test)]\npub mod a_tests;\n",
            "src/a.rs": "pub struct S;\n",
            "src/a_tests.rs": "pub struct TestOnly;\n",
        })
        self.assertEqual(common_admission.collect_pub_items(crate), {"S"})

    def test_path_tests_mod_excluded(self) -> None:
        crate = _make_crate(self, {
            "src/lib.rs": '#[path = "tests.rs"]\nmod tests;\n',
            "src/tests.rs": "pub struct TestOnly;\n",
        })
        self.assertEqual(common_admission.collect_pub_items(crate), set())

    def test_inline_tests_block_excluded(self) -> None:
        crate = _make_crate(self, {
            "src/lib.rs": "pub mod a { pub struct In; }\nmod tests { pub struct Y; }\n",
        })
        self.assertEqual(common_admission.collect_pub_items(crate), {"In"})

    def test_private_mod_items_excluded(self) -> None:
        crate = _make_crate(self, {
            "src/lib.rs": "mod hidden { pub struct Hidden; }\npub mod open;\n",
            "src/open.rs": "pub struct Shown;\n",
        })
        self.assertEqual(common_admission.collect_pub_items(crate), {"Shown"})

    def test_type_alias_and_shared_wrapper(self) -> None:
        crate = _make_crate(self, {
            "src/lib.rs": "pub mod a;\n",
            "src/a.rs": (
                "pub type Alias = u8;\n"
                "pub type SharedFoo = Arc<dyn Foo>;\n"
                "pub type SharedBar = std::sync::Arc<Baz>;\n"
                "pub type SharedBaz = tokio::sync::Mutex<u8>;\n"
            ),
        })
        self.assertEqual(
            common_admission.collect_pub_items(crate), {"Alias", "SharedBaz"}
        )

    def test_cfg_test_type_alias_excluded(self) -> None:
        crate = _make_crate(self, {
            "src/lib.rs": "pub mod a;\n",
            "src/a.rs": "#[cfg(test)]\npub type TestAlias = u8;\n",
        })
        self.assertEqual(common_admission.collect_pub_items(crate), set())

    def test_fn_const_use_excluded(self) -> None:
        crate = _make_crate(self, {
            "src/lib.rs": "pub mod a;\n",
            "src/a.rs": "pub fn f() {}\npub const C: u8 = 1;\npub use crate::f;\n",
        })
        self.assertEqual(common_admission.collect_pub_items(crate), set())

    def test_missing_lib_entry_fails_loud(self) -> None:
        crate = _make_crate(self, {"src/other.rs": "pub struct S;\n"})
        with self.assertRaises(common_admission.CommonAdmissionError):
            common_admission.collect_pub_items(crate)


class CompareTest(unittest.TestCase):
    def test_missing_in_doc(self) -> None:
        missing_doc, missing_code = common_admission.compare({"A"}, {"A", "B"})
        self.assertEqual(missing_doc, ["B"])
        self.assertEqual(missing_code, [])
        self.assertEqual(
            common_admission.deviation_items(missing_doc, missing_code),
            ["missing-in-doc: B"],
        )

    def test_missing_in_code(self) -> None:
        missing_doc, missing_code = common_admission.compare({"A", "B"}, {"A"})
        self.assertEqual(missing_doc, [])
        self.assertEqual(missing_code, ["B"])
        self.assertEqual(
            common_admission.deviation_items(missing_doc, missing_code),
            ["missing-in-code: B"],
        )

    def test_consistent(self) -> None:
        missing_doc, missing_code = common_admission.compare({"A", "B"}, {"B", "A"})
        self.assertEqual((missing_doc, missing_code), ([], []))
        self.assertEqual(common_admission.deviation_items([], []), [])

    def test_deviation_items_sorted(self) -> None:
        items = common_admission.deviation_items(["B", "A"], ["Z", "Y"])
        self.assertEqual(
            items,
            [
                "missing-in-code: Y",
                "missing-in-code: Z",
                "missing-in-doc: A",
                "missing-in-doc: B",
            ],
        )


class FindDeviationsTest(unittest.TestCase):
    def test_end_to_end(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            doc_dir = root / "docs" / "design" / "common"
            doc_dir.mkdir(parents=True)
            (doc_dir / "shared-types.md").write_text("### Foo\n", encoding="utf-8")
            (doc_dir / "core-traits.md").write_text("", encoding="utf-8")
            crate = root / "crates" / "common"
            (crate / "src").mkdir(parents=True)
            (crate / "src" / "lib.rs").write_text("pub mod foo;\n", encoding="utf-8")
            (crate / "src" / "foo.rs").write_text(
                "pub struct Foo;\npub struct Bar;\n", encoding="utf-8"
            )
            edge_data = dep_edges.EdgeData(
                edges=[],
                dir_to_pkg={"common": "closeclaw-common"},
                root_dir="common",
                dir_to_path={"common": str(crate)},
            )
            items = common_admission.find_deviations(root, edge_data)
        self.assertEqual(items, ["missing-in-doc: Bar"])

    def test_common_crate_missing_fails_loud(self) -> None:
        edge_data = dep_edges.EdgeData(
            edges=[], dir_to_pkg={}, root_dir="", dir_to_path={}
        )
        with self.assertRaises(second_export.SecondExportError):
            common_admission.find_deviations(Path("."), edge_data)


if __name__ == "__main__":
    unittest.main()
