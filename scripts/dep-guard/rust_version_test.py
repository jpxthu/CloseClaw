#!/usr/bin/env python3
"""rust_version 单元测试（workspace 成员解析与 rust-version 声明判定）。"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest import mock

import dep_guard
import rust_version

GOOD_MEMBER = (
    "[package]\n"
    'name = "closeclaw-demo"\n'
    'version = "0.1.0"\n'
    "edition.workspace = true\n"
    "rust-version.workspace = true\n"
)

BROKEN_TOML = "[workspace\nmembers = ["


def _make_workspace(case: unittest.TestCase, files: dict[str, str]) -> Path:
    """在临时目录构造 workspace 树（key 为相对仓库根路径），返回仓库根目录。

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


def _root_manifest(members: list[str], exclude: list[str] | None = None) -> str:
    entry_list = ", ".join(f'"{m}"' for m in members)
    lines = ["[workspace]", f"members = [{entry_list}]", 'resolver = "3"']
    if exclude is not None:
        exclude_list = ", ".join(f'"{m}"' for m in exclude)
        lines.append(f"exclude = [{exclude_list}]")
    lines += ["", "[workspace.package]", 'rust-version = "1.85"', ""]
    return "\n".join(lines)


def _member(rel: str, manifest: str = GOOD_MEMBER) -> dict[str, str]:
    return {f"{rel}/Cargo.toml": manifest}


class WorkspaceMembersTest(unittest.TestCase):
    """workspace_members：成员解析、glob 展开、exclude 剔除与异常分支。"""

    def test_members_sorted_with_absolute_paths(self) -> None:
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/b", "crates/a"]),
            **_member("crates/a"),
            **_member("crates/b"),
        })
        members = rust_version.workspace_members(root)
        self.assertEqual([rel for rel, _ in members], ["crates/a", "crates/b"])
        self.assertEqual(members[0][1], root / "crates" / "a")
        self.assertEqual(members[1][1], root / "crates" / "b")

    def test_glob_members_keep_only_manifest_dirs(self) -> None:
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/*"]),
            **_member("crates/kept"),
            "crates/no_manifest/placeholder.txt": "",
            "crates/loose_file.txt": "",
        })
        members = rust_version.workspace_members(root)
        self.assertEqual([rel for rel, _ in members], ["crates/kept"])

    def test_exclude_exact_entry_removed(self) -> None:
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/*"], exclude=["crates/skip"]),
            **_member("crates/kept"),
            **_member("crates/skip"),
        })
        members = rust_version.workspace_members(root)
        self.assertEqual([rel for rel, _ in members], ["crates/kept"])

    def test_exclude_glob_entry_removed(self) -> None:
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/*"], exclude=["crates/legacy-*"]),
            **_member("crates/kept"),
            **_member("crates/legacy-old"),
        })
        members = rust_version.workspace_members(root)
        self.assertEqual([rel for rel, _ in members], ["crates/kept"])

    def test_duplicate_entries_deduped_and_normalized(self) -> None:
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/a", "./crates/a"]),
            **_member("crates/a"),
        })
        members = rust_version.workspace_members(root)
        self.assertEqual([rel for rel, _ in members], ["crates/a"])

    def test_empty_members_returns_empty(self) -> None:
        root = _make_workspace(self, {"Cargo.toml": _root_manifest([])})
        self.assertEqual(rust_version.workspace_members(root), [])

    def test_missing_root_manifest_raises(self) -> None:
        root = _make_workspace(self, {})
        with self.assertRaises(rust_version.RustVersionError):
            rust_version.workspace_members(root)

    def test_broken_root_manifest_raises(self) -> None:
        root = _make_workspace(self, {"Cargo.toml": BROKEN_TOML})
        with self.assertRaises(rust_version.RustVersionError):
            rust_version.workspace_members(root)

    def test_non_string_member_entry_raises(self) -> None:
        root = _make_workspace(self, {"Cargo.toml": '[workspace]\nmembers = [1]\n'})
        with self.assertRaises(rust_version.RustVersionError):
            rust_version.workspace_members(root)


class FindViolationsTest(unittest.TestCase):
    """find_violations：workspace 继承判定、硬编码拒绝与排序确定性。"""

    def test_all_workspace_inherited_no_violations(self) -> None:
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/a", "crates/b"]),
            **_member("crates/a"),
            **_member("crates/b"),
        })
        self.assertEqual(rust_version.find_violations(root), [])

    def test_missing_declaration_violation(self) -> None:
        manifest = (
            "[package]\n"
            'name = "closeclaw-demo"\n'
            'version = "0.1.0"\n'
            "edition.workspace = true\n"
        )
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/a"]),
            **_member("crates/a", manifest),
        })
        violations = rust_version.find_violations(root)
        self.assertEqual(len(violations), 1)
        path, reason = violations[0]
        self.assertEqual(path, "crates/a")
        self.assertIn("未声明", reason)

    def test_hardcoded_version_violation(self) -> None:
        manifest = GOOD_MEMBER.replace(
            "rust-version.workspace = true", 'rust-version = "1.82"'
        )
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/a"]),
            **_member("crates/a", manifest),
        })
        violations = rust_version.find_violations(root)
        self.assertEqual(len(violations), 1)
        path, reason = violations[0]
        self.assertEqual(path, "crates/a")
        self.assertIn("硬编码", reason)
        self.assertIn("1.82", reason)

    def test_inline_table_workspace_true_passes(self) -> None:
        manifest = GOOD_MEMBER.replace(
            "rust-version.workspace = true", "rust-version = { workspace = true }"
        )
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/a"]),
            **_member("crates/a", manifest),
        })
        self.assertEqual(rust_version.find_violations(root), [])

    def test_inline_table_workspace_false_violation(self) -> None:
        manifest = GOOD_MEMBER.replace(
            "rust-version.workspace = true", "rust-version = { workspace = false }"
        )
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/a"]),
            **_member("crates/a", manifest),
        })
        violations = rust_version.find_violations(root)
        self.assertEqual(len(violations), 1)
        self.assertIn("硬编码", violations[0][1])

    def test_inline_table_non_workspace_key_violation(self) -> None:
        manifest = GOOD_MEMBER.replace(
            "rust-version.workspace = true", "rust-version = { other = true }"
        )
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/a"]),
            **_member("crates/a", manifest),
        })
        violations = rust_version.find_violations(root)
        self.assertEqual(len(violations), 1)
        self.assertIn("硬编码", violations[0][1])

    def test_member_manifest_missing_raises(self) -> None:
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/ghost"]),
        })
        with self.assertRaises(rust_version.RustVersionError):
            rust_version.find_violations(root)

    def test_member_manifest_broken_raises(self) -> None:
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/a"]),
            "crates/a/Cargo.toml": BROKEN_TOML,
        })
        with self.assertRaises(rust_version.RustVersionError):
            rust_version.find_violations(root)

    def test_violations_sorted_deterministic(self) -> None:
        hardcoded = GOOD_MEMBER.replace(
            "rust-version.workspace = true", 'rust-version = "1.82"'
        )
        missing = "\n".join(
            line for line in GOOD_MEMBER.splitlines() if not line.startswith("rust-version")
        ) + "\n"
        root = _make_workspace(self, {
            "Cargo.toml": _root_manifest(["crates/b", "crates/a"]),
            **_member("crates/a", missing),
            **_member("crates/b", hardcoded),
        })
        violations = rust_version.find_violations(root)
        self.assertEqual([path for path, _ in violations], ["crates/a", "crates/b"])

    def test_empty_members_no_violations(self) -> None:
        root = _make_workspace(self, {"Cargo.toml": _root_manifest([])})
        self.assertEqual(rust_version.find_violations(root), [])


class CheckRustVersionDeclaredTest(unittest.TestCase):
    """check_rust_version_declared：编排层 baseline diff 与报文语义。

    参照 dep_guard_test.py CheckSecondExportsDiffTest：mock 扫描层
    （find_violations），baseline 经临时目录真实加载，走真实 diff 链路。
    """

    UNDECLARED = ("crates/a", "rust-version 未声明")
    HARDCODED = ('crates/b', 'rust-version 硬编码 "1.82"（须 rust-version.workspace = true）')

    def _run_check(
        self, tmp: str, baseline_content: str, violations: list[tuple[str, str]]
    ) -> dep_guard.CheckResult:
        baseline_dir = Path(tmp)
        (baseline_dir / "rust-version-declared.txt").write_text(
            baseline_content, encoding="utf-8"
        )
        with (
            mock.patch.object(rust_version, "find_violations", return_value=violations),
            mock.patch.object(dep_guard, "BASELINE_DIR", baseline_dir),
        ):
            return dep_guard.check_rust_version_declared(Path("."))

    def test_no_violations_empty_baseline_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            result = self._run_check(tmp, "# 空清单\n", [])
        self.assertEqual(result.status, dep_guard.STATUS_PASS)
        summary = result.lines[-1]
        self.assertIn("违规 0 条", summary)
        self.assertIn("baseline 0 条", summary)

    def test_two_violations_fail_with_reasons(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            result = self._run_check(tmp, "", [self.UNDECLARED, self.HARDCODED])
        self.assertEqual(result.status, dep_guard.STATUS_FAIL)
        fail_lines = [line for line in result.lines if "[FAIL] 新增违规" in line]
        self.assertEqual(len(fail_lines), 2)
        self.assertIn("crates/a", fail_lines[0])
        self.assertIn("未声明", fail_lines[0])
        self.assertIn("crates/b", fail_lines[1])
        self.assertIn("硬编码", fail_lines[1])
        self.assertIn('"1.82"', fail_lines[1])

    def test_baseline_exempt_not_reported_and_eliminated_shown(self) -> None:
        """豁免条命中 → 不报；baseline 既有条目已消除 → [info] 可收窄。"""
        exempt = ("crates/exempt", "rust-version 未声明")
        eliminated = "crates/fixed"
        with tempfile.TemporaryDirectory() as tmp:
            result = self._run_check(
                tmp,
                f"{eliminated}\n{exempt[0]}\n",
                [self.UNDECLARED, exempt],
            )
        self.assertEqual(result.status, dep_guard.STATUS_FAIL)
        fail_lines = [line for line in result.lines if "[FAIL] 新增违规" in line]
        self.assertEqual(len(fail_lines), 1)
        self.assertIn("crates/a", fail_lines[0])
        self.assertFalse(any("crates/exempt" in line for line in fail_lines))
        self.assertTrue(
            any(f"可收窄 baseline（rust-version 已声明）: {eliminated}" in line
                for line in result.lines)
        )

    def test_scan_error_single_fail_line(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            baseline_dir = Path(tmp)
            (baseline_dir / "rust-version-declared.txt").write_text("", encoding="utf-8")
            with (
                mock.patch.object(
                    rust_version,
                    "find_violations",
                    side_effect=rust_version.RustVersionError("成员 Cargo.toml 缺失"),
                ),
                mock.patch.object(dep_guard, "BASELINE_DIR", baseline_dir),
            ):
                result = dep_guard.check_rust_version_declared(Path("."))
        self.assertEqual(result.status, dep_guard.STATUS_FAIL)
        self.assertEqual(
            [line for line in result.lines if "[FAIL]" in line],
            ["  [FAIL] 成员 Cargo.toml 缺失"],
        )


class RustVersionCliTest(unittest.TestCase):
    """CLI：--only rust-version-declared 退出码 = FAIL 项数；全量汇总含本项。"""

    def _invoke(
        self, args: list[str], violations: list[tuple[str, str]]
    ):
        from click.testing import CliRunner

        with tempfile.TemporaryDirectory() as tmp:
            baseline_dir = Path(tmp)
            (baseline_dir / "rust-version-declared.txt").write_text("", encoding="utf-8")
            with (
                mock.patch.object(
                    rust_version, "find_violations", return_value=violations
                ),
                mock.patch.object(dep_guard, "BASELINE_DIR", baseline_dir),
            ):
                result = CliRunner().invoke(dep_guard.main, args)
        return result

    def test_only_pass_exit_zero(self) -> None:
        result = self._invoke(["check", "--only", "rust-version-declared"], [])
        self.assertEqual(result.exit_code, 0)
        self.assertIn("rust-version-declared=PASS", result.output)

    def test_only_fail_exit_one(self) -> None:
        result = self._invoke(
            ["check", "--only", "rust-version-declared"],
            [("crates/a", "rust-version 未声明")],
        )
        self.assertEqual(result.exit_code, 1)

    def test_full_check_summary_contains_pass(self) -> None:
        """全量 check：其余项以假 runner 置 PASS，汇总行含本项 PASS。"""
        originals = dict(dep_guard.CHECK_RUNNERS)
        dep_guard.CHECK_RUNNERS.update({
            check_id: (lambda repo_root: dep_guard.CheckResult(
                check_id, dep_guard.STATUS_PASS, []))
            for check_id in originals
            if check_id != "rust-version-declared"
        })
        try:
            result = self._invoke(["check"], [])
        finally:
            dep_guard.CHECK_RUNNERS.clear()
            dep_guard.CHECK_RUNNERS.update(originals)
        self.assertEqual(result.exit_code, 0)
        self.assertIn("rust-version-declared=PASS", result.output)


if __name__ == "__main__":
    unittest.main()
