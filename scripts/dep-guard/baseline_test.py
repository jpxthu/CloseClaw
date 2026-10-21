#!/usr/bin/env python3
"""baseline 单元测试：加载语义（load_baseline 跳过注释/空行）+ diff 判定（key 归一化与 None 透传）+ 收窄写回（remove_entries 行过滤删行）。"""

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


class RemoveEntriesTest(unittest.TestCase):
    def _write(self, tmp: str, content: str) -> Path:
        path = Path(tmp) / "baseline.txt"
        path.write_text(content, encoding="utf-8")
        return path

    def test_removes_target_keeps_header_and_order(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write(tmp, "# 头部注释\n\na -> b\nb -> c\nc -> d\n")
            removed = baseline.remove_entries(path, ["b -> c"])
            self.assertEqual(removed, ["b -> c"])
            self.assertEqual(
                path.read_text(encoding="utf-8"), "# 头部注释\n\na -> b\nc -> d\n"
            )

    def test_empty_entries_noop(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write(tmp, "# 头部注释\n\na -> b\nb -> c\n")
            self.assertEqual(baseline.remove_entries(path, []), [])
            self.assertEqual(path.read_text(encoding="utf-8"), "# 头部注释\n\na -> b\nb -> c\n")

    def test_missing_entry_noop_without_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write(tmp, "# 头部注释\n\na -> b\n")
            self.assertEqual(baseline.remove_entries(path, ["x -> y"]), [])
            self.assertEqual(path.read_text(encoding="utf-8"), "# 头部注释\n\na -> b\n")

    def test_duplicate_lines_all_removed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write(tmp, "a -> b\na -> b\nc -> d\n")
            self.assertEqual(baseline.remove_entries(path, ["a -> b"]), ["a -> b"])
            self.assertEqual(path.read_text(encoding="utf-8"), "c -> d\n")

    def test_reload_after_narrow_keeps_semantics(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write(tmp, "# 头部注释\n\n  a -> b  \nb -> c\n")
            self.assertEqual(baseline.load_baseline(path), ["a -> b", "b -> c"])
            removed = baseline.remove_entries(path, ["a -> b"])
            self.assertEqual(removed, ["a -> b"])
            self.assertEqual(baseline.load_baseline(path), ["b -> c"])


if __name__ == "__main__":
    unittest.main()
