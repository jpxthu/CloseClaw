#!/usr/bin/env python3
"""rust_version 单元测试（workspace 成员解析与 rust-version 声明判定）。"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

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


if __name__ == "__main__":
    unittest.main()
