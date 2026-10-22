#!/usr/bin/env python3
"""test_naming_guard 单元测试：扫描器（正常路径 / 边界）+ baseline 三态与收窄 +
CLI 错误路径（非法参数 / 路径不存在）。
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from click.testing import CliRunner

import test_naming_guard as tng


# ---------- 扫描器：正常路径 -----------------------------------------------------


class ScannerNormalTest(unittest.TestCase):
    def _scan(self, content: str) -> list[tuple[int, str]]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "t.rs"
            path.write_text(content, encoding="utf-8")
            return tng.scan_file(path)

    def test_test_prefix_passes(self) -> None:
        self.assertEqual(self._scan("#[test]\nfn test_foo() {}\n"), [])

    def test_plain_test_attr_captures_missing_prefix(self) -> None:
        self.assertEqual(self._scan("#[test]\nfn bad_name() {}\n"), [(2, "bad_name")])

    def test_tokio_test_attr_captures_missing_prefix(self) -> None:
        content = "#[tokio::test]\nasync fn bad_name() {}\n"
        self.assertEqual(self._scan(content), [(2, "bad_name")])

    def test_tokio_test_prefixed_passes(self) -> None:
        content = "#[tokio::test]\nasync fn test_foo() {}\n"
        self.assertEqual(self._scan(content), [])


# ---------- 扫描器：边界 ---------------------------------------------------------


class ScannerBoundaryTest(unittest.TestCase):
    def _scan(self, content: str) -> list[tuple[int, str]]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "t.rs"
            path.write_text(content, encoding="utf-8")
            return tng.scan_file(path)

    def test_cfg_test_attr_not_a_test_attribute(self) -> None:
        self.assertEqual(self._scan("#[cfg(test)]\nfn bad_name() {}\n"), [])

    def test_test_case_attr_not_a_test_attribute(self) -> None:
        self.assertEqual(self._scan("#[test_case(1)]\nfn bad_name() {}\n"), [])

    def test_test_suite_attr_not_a_test_attribute(self) -> None:
        self.assertEqual(self._scan("#[test_suite]\nfn bad_name() {}\n"), [])

    def test_plain_fn_without_attr_not_flagged(self) -> None:
        self.assertEqual(self._scan("fn bad_name() {}\n"), [])

    def test_should_panic_between_attr_and_fn_still_captured(self) -> None:
        content = "#[test]\n#[should_panic]\nfn bad_name() {}\n"
        self.assertEqual(self._scan(content), [(3, "bad_name")])

    def test_should_panic_with_args_still_captured(self) -> None:
        content = '#[test]\n#[should_panic(expected = "boom")]\nfn bad_name() {}\n'
        self.assertEqual(self._scan(content), [(3, "bad_name")])

    def test_async_fn_captured(self) -> None:
        content = "#[test]\nasync fn bad_name() {}\n"
        self.assertEqual(self._scan(content), [(2, "bad_name")])

    def test_brace_after_attr_no_crash_no_violation(self) -> None:
        content = "#[test]\n}\nfn test_ok() {}\n"
        self.assertEqual(self._scan(content), [])

    def test_attr_at_eof_no_crash(self) -> None:
        self.assertEqual(self._scan("fn test_ok() {}\n#[test]"), [])

    def test_indented_fn_inside_mod_captured(self) -> None:
        content = "mod tests {\n    #[test]\n    fn bad_name() {}\n}\n"
        self.assertEqual(self._scan(content), [(3, "bad_name")])

    def test_pub_fn_captured(self) -> None:
        content = "#[test]\npub fn bad_name() {}\n"
        self.assertEqual(self._scan(content), [(2, "bad_name")])

    def test_blank_and_comment_lines_between_attr_and_fn_captured(self) -> None:
        content = "#[test]\n// note\n\nfn bad_name() {}\n"
        self.assertEqual(self._scan(content), [(4, "bad_name")])

    def test_generic_fn_name_captured(self) -> None:
        content = "#[test]\nfn bad_name<T>() {}\n"
        self.assertEqual(self._scan(content), [(2, "bad_name")])

    def test_non_test_file_zero_violations(self) -> None:
        content = (
            "pub struct Foo;\n\nimpl Foo {\n    pub fn build() -> Self {\n"
            "        Foo\n    }\n\n    fn helper(&self) -> usize {\n        0\n    }\n}\n"
        )
        self.assertEqual(self._scan(content), [])


# ---------- 扫描范围与条目格式 ---------------------------------------------------


class ScanRootTest(unittest.TestCase):
    def test_target_dir_pruned_and_entries_sorted(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "target").mkdir()
            (root / "target" / "gen.rs").write_text("#[test]\nfn bad_a() {}\n", encoding="utf-8")
            (root / "src").mkdir()
            (root / "src" / "b.rs").write_text("#[test]\nfn bad_b() {}\n", encoding="utf-8")
            (root / "src" / "a.rs").write_text("#[test]\nfn bad_c() {}\n", encoding="utf-8")
            entries = tng.scan_root(root)
        self.assertEqual(
            entries,
            [
                "src/a.rs:2: missing_test_prefix(bad_c)",
                "src/b.rs:2: missing_test_prefix(bad_b)",
            ],
        )

    def test_strip_key_removes_lineno_only(self) -> None:
        self.assertEqual(
            tng.strip_key("crates/x/src/a.rs:10: missing_test_prefix(foo)"),
            "crates/x/src/a.rs: missing_test_prefix(foo)",
        )


# ---------- baseline 三态与收窄 --------------------------------------------------


class DiffBaselineTest(unittest.TestCase):
    def test_new_item_fails(self) -> None:
        diff = tng.diff_baseline([], ["a.rs:1: missing_test_prefix(foo)"])
        self.assertTrue(diff.has_new)
        self.assertEqual(diff.new_items, ["a.rs:1: missing_test_prefix(foo)"])
        self.assertEqual(diff.eliminated_items, [])

    def test_exempt_item_neither_new_nor_eliminated(self) -> None:
        entry = "a.rs:1: missing_test_prefix(foo)"
        diff = tng.diff_baseline([entry], [entry])
        self.assertFalse(diff.has_new)
        self.assertEqual(diff.eliminated_items, [])

    def test_eliminated_item_reported(self) -> None:
        diff = tng.diff_baseline(["a.rs:1: missing_test_prefix(foo)"], [])
        self.assertFalse(diff.has_new)
        self.assertEqual(diff.eliminated_items, ["a.rs:1: missing_test_prefix(foo)"])

    def test_lineno_drift_is_neither_new_nor_eliminated(self) -> None:
        diff = tng.diff_baseline(
            ["a.rs:1: missing_test_prefix(foo)"], ["a.rs:9: missing_test_prefix(foo)"]
        )
        self.assertFalse(diff.has_new)
        self.assertEqual(diff.eliminated_items, [])

    def test_renamed_fn_is_true_new_and_eliminated(self) -> None:
        diff = tng.diff_baseline(
            ["a.rs:1: missing_test_prefix(foo)"], ["a.rs:1: missing_test_prefix(bar)"]
        )
        self.assertTrue(diff.has_new)
        self.assertEqual(diff.new_items, ["a.rs:1: missing_test_prefix(bar)"])
        self.assertEqual(diff.eliminated_items, ["a.rs:1: missing_test_prefix(foo)"])

    def test_duplicate_keys_counted_as_multiset(self) -> None:
        """同键（同文件同名函数）多条目按多重集计数：2 实际 − 1 baseline = 1 新增，
        报告该键一条代表条目（保留行号定位，取首个代表）。"""
        base = ["a.rs:1: missing_test_prefix(foo)"]
        actual = ["a.rs:1: missing_test_prefix(foo)", "a.rs:5: missing_test_prefix(foo)"]
        diff = tng.diff_baseline(base, actual)
        self.assertTrue(diff.has_new)
        self.assertEqual(len(diff.new_items), 1)
        self.assertEqual(
            tng.strip_key(diff.new_items[0]), "a.rs: missing_test_prefix(foo)"
        )
        self.assertIn(diff.new_items[0], actual)


class BaselineFileTest(unittest.TestCase):
    def test_load_skips_comments_and_blank(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "b.txt"
            path.write_text("# 头\n\n  a.rs:1: x  \n# 另一条\na.rs:2: y\n", encoding="utf-8")
            self.assertEqual(tng.load_baseline(path), ["a.rs:1: x", "a.rs:2: y"])

    def test_write_baseline_writes_header_and_sorted_entries(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "baselines" / "b.txt"
            tng.write_baseline(path, ["b.rs:2: x", "a.rs:1: y"])
            lines = path.read_text(encoding="utf-8").splitlines()
        self.assertEqual(lines[0], "# test-naming-guard baseline — 存量豁免清单（自动生成）")
        self.assertTrue(all(line.startswith("#") for line in lines[:4]))
        self.assertEqual(lines[4:], ["a.rs:1: y", "b.rs:2: x"])

    def test_remove_entries_narrows_keeps_header_and_order(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "b.txt"
            path.write_text("# 头\n\na.rs:1: x\na.rs:2: y\na.rs:3: z\n", encoding="utf-8")
            removed = tng.remove_entries(path, ["a.rs:2: y"])
            self.assertEqual(removed, ["a.rs:2: y"])
            self.assertEqual(
                path.read_text(encoding="utf-8"), "# 头\n\na.rs:1: x\na.rs:3: z\n"
            )

    def test_remove_entries_missing_target_noop(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "b.txt"
            path.write_text("# 头\n\na.rs:1: x\n", encoding="utf-8")
            self.assertEqual(tng.remove_entries(path, ["a.rs:9: nope"]), [])
            self.assertEqual(path.read_text(encoding="utf-8"), "# 头\n\na.rs:1: x\n")


# ---------- CLI：错误路径与状态转换 -----------------------------------------------


class CliTest(unittest.TestCase):
    def _setup(self, baseline_text: str, files: dict[str, str]) -> tuple[Path, Path]:
        """构建临时扫描根 + baseline，返回 (root, baseline_path)。"""
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(_cleanup_dir, tmp)
        for rel, content in files.items():
            path = tmp / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        baseline_path = tmp / "baselines" / "missing_test_prefix.txt"
        baseline_path.parent.mkdir(parents=True, exist_ok=True)
        baseline_path.write_text(baseline_text, encoding="utf-8")
        return tmp, baseline_path

    def _invoke(self, root: Path, baseline_path: Path, args: list[str]):
        with mock.patch.object(tng, "BASELINE_PATH", baseline_path):
            return CliRunner().invoke(tng.main, ["--root", str(root), *args])

    def test_unknown_option_exit_two(self) -> None:
        root, baseline_path = self._setup("", {})
        result = self._invoke(root, baseline_path, ["--bogus"])
        self.assertEqual(result.exit_code, 2)

    def test_nonexistent_root_exit_two(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            baseline_path = Path(tmp) / "b.txt"
            baseline_path.write_text("", encoding="utf-8")
            with mock.patch.object(tng, "BASELINE_PATH", baseline_path):
                result = CliRunner().invoke(
                    tng.main, ["--root", str(Path(tmp) / "nope")]
                )
        self.assertEqual(result.exit_code, 2)

    def test_update_baseline_and_fix_conflict_exit_two(self) -> None:
        root, baseline_path = self._setup("", {})
        result = self._invoke(root, baseline_path, ["--update-baseline", "--fix"])
        self.assertEqual(result.exit_code, 2)

    def test_new_violation_fails_exit_one(self) -> None:
        root, baseline_path = self._setup("", {"src/a.rs": "#[test]\nfn bad() {}\n"})
        result = self._invoke(root, baseline_path, [])
        self.assertEqual(result.exit_code, 1)
        self.assertIn(
            "[FAIL] 新增违规: src/a.rs:2: missing_test_prefix(bad)", result.output
        )

    def test_exempt_item_passes_exit_zero(self) -> None:
        root, baseline_path = self._setup(
            "# 头\nsrc/a.rs:2: missing_test_prefix(bad)\n",
            {"src/a.rs": "#[test]\nfn bad() {}\n"},
        )
        result = self._invoke(root, baseline_path, [])
        self.assertEqual(result.exit_code, 0)
        self.assertIn("baseline 豁免 1", result.output)
        self.assertIn("→ PASS", result.output)

    def test_eliminated_item_info_without_fail(self) -> None:
        root, baseline_path = self._setup(
            "# 头\nsrc/a.rs:2: missing_test_prefix(old)\n", {}
        )
        result = self._invoke(root, baseline_path, [])
        self.assertEqual(result.exit_code, 0)
        self.assertIn("[info] 可收窄 baseline（违规已消除）: src/a.rs:2:", result.output)

    def test_fix_narrows_baseline_file(self) -> None:
        root, baseline_path = self._setup(
            "# 头\nsrc/a.rs:2: missing_test_prefix(old)\n", {}
        )
        result = self._invoke(root, baseline_path, ["--fix"])
        self.assertEqual(result.exit_code, 0)
        self.assertIn(
            "[fix] 已收窄 baseline（违规已消除）: src/a.rs:2: missing_test_prefix(old)",
            result.output,
        )
        self.assertEqual(baseline_path.read_text(encoding="utf-8"), "# 头\n")

    def test_fix_keeps_fail_exit_code_with_new_violations(self) -> None:
        root, baseline_path = self._setup(
            "# 头\nsrc/a.rs:2: missing_test_prefix(old)\n",
            {"src/a.rs": "#[test]\nfn bad() {}\n"},
        )
        result = self._invoke(root, baseline_path, ["--fix"])
        self.assertEqual(result.exit_code, 1)
        self.assertIn("missing_test_prefix(bad)", result.output)

    def test_update_baseline_rewrites_from_full_scan(self) -> None:
        root, baseline_path = self._setup(
            "# 头\nsrc/stale.rs:9: missing_test_prefix(stale)\n",
            {"src/a.rs": "#[test]\nfn bad() {}\n"},
        )
        result = self._invoke(root, baseline_path, ["--update-baseline"])
        self.assertEqual(result.exit_code, 0)
        lines = baseline_path.read_text(encoding="utf-8").splitlines()
        self.assertEqual(lines[4:], ["src/a.rs:2: missing_test_prefix(bad)"])
        self.assertIn("（1 条）", result.output)


def _cleanup_dir(path: Path) -> None:
    import shutil

    shutil.rmtree(path, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
