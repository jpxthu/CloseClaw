#!/usr/bin/env python3
"""baseline diff 引擎单元测试（key 归一化判定语义与 None 透传）。"""

from __future__ import annotations

import unittest

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


if __name__ == "__main__":
    unittest.main()
