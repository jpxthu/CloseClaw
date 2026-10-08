#!/usr/bin/env python3
"""second_export 单元测试（公共可达性、use 树解析、源分类与豁免）。"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import second_export


def _make_crate(files: dict[str, str]) -> Path:
    """在临时目录构造 crate 树（key 为相对 crate 目录路径），返回 crate 目录。"""
    tmp = Path(tempfile.mkdtemp())
    for rel, content in files.items():
        path = tmp / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    return tmp


def _scan(files: dict[str, str]) -> list[str]:
    crate_root = _make_crate(files)
    return second_export.crate_second_exports(crate_root, "closeclaw-demo")


class PublicReachabilityTest(unittest.TestCase):
    def test_pub_mod_chain_reachable(self) -> None:
        items = _scan({
            "src/lib.rs": "pub mod inner;\n",
            "src/inner.rs": "pub use closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, ["closeclaw-demo: src/inner.rs:1 closeclaw_common::Foo"])

    def test_private_mod_unreachable(self) -> None:
        items = _scan({
            "src/lib.rs": "mod inner;\n",
            "src/inner.rs": "pub use closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, [])

    def test_pub_crate_restricted_mod_unreachable(self) -> None:
        items = _scan({
            "src/lib.rs": "pub(crate) mod inner;\n",
            "src/inner.rs": "pub use closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, [])

    def test_cfg_test_mod_subtree_unreachable(self) -> None:
        items = _scan({
            "src/lib.rs": "#[cfg(test)]\nmod tests;\n",
            "src/tests.rs": "pub use closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, [])

    def test_inline_pub_mod_reachable(self) -> None:
        items = _scan({
            "src/lib.rs": (
                "pub mod streaming {\n"
                "    pub use closeclaw_common::streaming::{Renderer, Sink};\n"
                "}\n"
            ),
        })
        self.assertEqual(
            items,
            [
                "closeclaw-demo: src/lib.rs:2 closeclaw_common::streaming::Renderer",
                "closeclaw-demo: src/lib.rs:2 closeclaw_common::streaming::Sink",
            ],
        )

    def test_inline_private_mod_unreachable(self) -> None:
        items = _scan({
            "src/lib.rs": (
                "mod hidden {\n"
                "    pub use closeclaw_common::Foo;\n"
                "}\n"
            ),
        })
        self.assertEqual(items, [])

    def test_private_mod_glob_reexport_propagates(self) -> None:
        items = _scan({
            "src/lib.rs": "mod hidden;\npub use hidden::*;\n",
            "src/hidden.rs": "pub use closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, ["closeclaw-demo: src/hidden.rs:1 closeclaw_common::Foo"])

    def test_private_mod_named_reexport_propagates(self) -> None:
        items = _scan({
            "src/lib.rs": "mod hidden;\npub use crate::hidden;\n",
            "src/hidden.rs": "pub use closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, ["closeclaw-demo: src/hidden.rs:1 closeclaw_common::Foo"])

    def test_no_public_path_without_lib(self) -> None:
        crate_root = _make_crate({"src/main.rs": "pub use closeclaw_common::Foo;\n"})
        self.assertEqual(
            second_export.crate_second_exports(crate_root, "closeclaw-demo"), []
        )


class UseStatementTest(unittest.TestCase):
    def test_multiline_brace_use_expands_leaves(self) -> None:
        items = _scan({
            "src/lib.rs": (
                "pub use closeclaw_common::processor::{\n"
                "    DslInstruction,\n"
                "    ProcessedMessage,\n"
                "};\n"
            ),
        })
        self.assertEqual(
            items,
            [
                "closeclaw-demo: src/lib.rs:1 closeclaw_common::processor::DslInstruction",
                "closeclaw-demo: src/lib.rs:1 closeclaw_common::processor::ProcessedMessage",
            ],
        )

    def test_nested_use_tree(self) -> None:
        leaves = second_export.parse_use_leaves("closeclaw_common::a::{b::{c, d}, e}")
        self.assertEqual(
            leaves,
            [
                ("closeclaw_common::a::b::c", None),
                ("closeclaw_common::a::b::d", None),
                ("closeclaw_common::a::e", None),
            ],
        )

    def test_alias_renamed(self) -> None:
        items = _scan({
            "src/lib.rs": "pub use closeclaw_common::Foo as Bar;\n",
        })
        self.assertEqual(items, ["closeclaw-demo: src/lib.rs:1 closeclaw_common::Foo as Bar"])

    def test_alias_inside_tree(self) -> None:
        items = _scan({
            "src/lib.rs": "pub use closeclaw_common::{Foo as Bar, Baz};\n",
        })
        self.assertEqual(
            items,
            [
                "closeclaw-demo: src/lib.rs:1 closeclaw_common::Baz",
                "closeclaw-demo: src/lib.rs:1 closeclaw_common::Foo as Bar",
            ],
        )

    def test_glob_use_flagged(self) -> None:
        items = _scan({
            "src/lib.rs": "pub use closeclaw_common::module::*;\n",
        })
        self.assertEqual(
            items, ["closeclaw-demo: src/lib.rs:1 closeclaw_common::module::*"]
        )

    def test_pub_crate_use_not_flagged(self) -> None:
        items = _scan({
            "src/lib.rs": "pub(crate) use closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, [])

    def test_private_use_not_flagged(self) -> None:
        items = _scan({
            "src/lib.rs": "use closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, [])

    def test_cfg_test_use_not_flagged(self) -> None:
        items = _scan({
            "src/lib.rs": "#[cfg(test)]\npub use closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, [])


class ExemptionTest(unittest.TestCase):
    def test_own_crate_sources_exempt(self) -> None:
        items = _scan({
            "src/lib.rs": (
                "pub mod foo;\n"
                "pub use crate::foo::Bar;\n"
                "pub use self::foo;\n"
                "pub use serde_json::Value;\n"
            ),
        })
        self.assertEqual(items, [])

    def test_shared_arc_type_alias_exempt(self) -> None:
        items = _scan({
            "src/lib.rs": "pub type SharedFoo = std::sync::Arc<closeclaw_common::Foo>;\n",
        })
        self.assertEqual(items, [])


class TypeAndExternTest(unittest.TestCase):
    def test_pub_type_alias_to_common_flagged(self) -> None:
        items = _scan({
            "src/lib.rs": "pub type Foo = closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, ["closeclaw-demo: src/lib.rs:1 closeclaw_common::Foo"])

    def test_pub_type_alias_shim_flagged(self) -> None:
        items = _scan({
            "src/lib.rs": "pub type Foo = crate::common::Foo;\n",
        })
        self.assertEqual(items, ["closeclaw-demo: src/lib.rs:1 crate::common::Foo"])

    def test_pub_extern_crate_flagged(self) -> None:
        items = _scan({
            "src/lib.rs": "pub extern crate closeclaw_common;\n",
        })
        self.assertEqual(items, ["closeclaw-demo: src/lib.rs:1 closeclaw_common"])

    def test_private_extern_crate_not_flagged(self) -> None:
        items = _scan({
            "src/lib.rs": "#[macro_use]\nextern crate closeclaw_common;\n",
        })
        self.assertEqual(items, [])


class ScannerRobustnessTest(unittest.TestCase):
    def test_strings_and_comments_do_not_confuse_depth(self) -> None:
        items = _scan({
            "src/lib.rs": (
                "// 注释 { pub use closeclaw_common::Nope;\n"
                "const S: &str = \"pub use closeclaw_common::Nope;\";\n"
                "pub struct S { pub field: u32 }\n"
                "pub use closeclaw_common::Foo;\n"
            ),
        })
        self.assertEqual(items, ["closeclaw-demo: src/lib.rs:4 closeclaw_common::Foo"])

    def test_item_after_fn_body_not_polluted(self) -> None:
        items = _scan({
            "src/lib.rs": (
                "pub fn f() -> u32 {\n"
                "    use std::collections::HashMap;\n"
                "    1\n"
                "}\n"
                "use closeclaw_common::Foo;\n"
            ),
        })
        self.assertEqual(items, [])

    def test_path_attr_mod_resolved(self) -> None:
        items = _scan({
            "src/lib.rs": "#[path = \"custom.rs\"]\npub mod inner;\n",
            "src/custom.rs": "pub use closeclaw_common::Foo;\n",
        })
        self.assertEqual(items, ["closeclaw-demo: src/custom.rs:1 closeclaw_common::Foo"])


class FindSecondExportsTest(unittest.TestCase):
    def test_common_dir_excluded_and_pkg_mapped(self) -> None:
        from dep_edges import Edge, EdgeData

        llm_root = _make_crate({
            "src/lib.rs": "pub use closeclaw_common::Foo;\n",
        })
        common_root = _make_crate({
            "src/lib.rs": "pub use closeclaw_common::Foo;\n",
        })
        edge_data = EdgeData(
            edges=[Edge("llm", "common")],
            dir_to_pkg={"llm": "closeclaw-llm", "common": "closeclaw-common"},
            root_dir="llm",
            dir_to_path={"llm": str(llm_root), "common": str(common_root)},
        )
        items = second_export.find_second_exports(edge_data)
        self.assertEqual(
            items, ["closeclaw-llm: src/lib.rs:1 closeclaw_common::Foo"]
        )

    def test_missing_common_pkg_fails_loud(self) -> None:
        from dep_edges import EdgeData

        edge_data = EdgeData(
            edges=[],
            dir_to_pkg={"llm": "closeclaw-llm"},
            root_dir="llm",
            dir_to_path={"llm": "/nowhere/llm"},
        )
        with self.assertRaises(second_export.SecondExportError):
            second_export.find_second_exports(edge_data)


if __name__ == "__main__":
    unittest.main()
