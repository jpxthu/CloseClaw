#!/usr/bin/env python3
"""baseline 单元测试：加载语义（load_baseline 跳过注释/空行）+ diff 判定（key 归一化与 None 透传）。"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import baseline
import second_export

BASE_ENTRY = "closeclaw-gateway: src/lib.rs:132 closeclaw_common::Foo"
DRIFTED_ENTRY = "closeclaw-gateway: src/lib.rs:136 closeclaw_common::Foo"


class DiffBaselineKeyTest(unittest.TestCase):
    def test_lineno_only_drift_is_neither_new_nor_eliminated(self) -> None:
        diff = baseline.diff_baseline(
            [BASE_ENTRY], [DRIFTED_ENTRY], key=second_export.strip_lineno
        )
        self.assertEqual(diff.new_items, [])
        self.assertEqual(diff.eliminated_items, [])
        self.assertFalse(diff.has_new)

    def test_key_none_matches_raw_set_diff(self) -> None:
        diff = baseline.diff_baseline([BASE_ENTRY], [DRIFTED_ENTRY])
        self.assertEqual(diff.new_items, [DRIFTED_ENTRY])
        self.assertEqual(diff.eliminated_items, [BASE_ENTRY])
        self.assertTrue(diff.has_new)

    def test_reports_original_entries_with_lineno(self) -> None:
        base = [
            BASE_ENTRY,
            "closeclaw-llm: src/lib.rs:1 closeclaw_common::Bar",
        ]
        actual = [
            DRIFTED_ENTRY,
            "closeclaw-llm: src/lib.rs:9 closeclaw_common::Baz",
        ]
        diff = baseline.diff_baseline(base, actual, key=second_export.strip_lineno)
        self.assertEqual(
            diff.new_items,
            ["closeclaw-llm: src/lib.rs:9 closeclaw_common::Baz"],
        )
        self.assertEqual(
            diff.eliminated_items,
            ["closeclaw-llm: src/lib.rs:1 closeclaw_common::Bar"],
        )

    def test_true_new_entry_still_fails_under_key(self) -> None:
        base = [BASE_ENTRY]
        actual = [DRIFTED_ENTRY, "closeclaw-llm: src/lib.rs:2 closeclaw_common::Qux"]
        diff = baseline.diff_baseline(base, actual, key=second_export.strip_lineno)
        self.assertEqual(
            diff.new_items, ["closeclaw-llm: src/lib.rs:2 closeclaw_common::Qux"]
        )
        self.assertTrue(diff.has_new)

    def test_multiple_entries_sorted_original_form(self) -> None:
        base = []
        actual = [
            "closeclaw-llm: src/b.rs:2 closeclaw_common::B",
            "closeclaw-llm: src/a.rs:1 closeclaw_common::A",
        ]
        diff = baseline.diff_baseline(base, actual, key=second_export.strip_lineno)
        self.assertEqual(
            diff.new_items,
            [
                "closeclaw-llm: src/a.rs:1 closeclaw_common::A",
                "closeclaw-llm: src/b.rs:2 closeclaw_common::B",
            ],
        )


class BaselineTest(unittest.TestCase):
    def test_load_skips_comments_and_blank(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "b.txt"
            path.write_text("# 注释\n\n  a -> b  \n# 另一条注释\na -> b\nb -> c\n", encoding="utf-8")
            items = baseline.load_baseline(path)
        self.assertEqual(items, ["a -> b", "b -> c"])

    def test_diff_new_items_fail(self) -> None:
        diff = baseline.diff_baseline(["a"], ["a", "b"])
        self.assertEqual(diff.new_items, ["b"])
        self.assertEqual(diff.eliminated_items, [])
        self.assertTrue(diff.has_new)

    def test_diff_eliminated_items_info(self) -> None:
        diff = baseline.diff_baseline(["a", "b"], ["a"])
        self.assertEqual(diff.new_items, [])
        self.assertEqual(diff.eliminated_items, ["b"])
        self.assertFalse(diff.has_new)

    def test_diff_consistent(self) -> None:
        diff = baseline.diff_baseline(["a", "b"], ["b", "a"])
        self.assertEqual(diff.new_items, [])
        self.assertEqual(diff.eliminated_items, [])


if __name__ == "__main__":
    unittest.main()
