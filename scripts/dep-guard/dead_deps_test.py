#!/usr/bin/env python3
"""dead_deps 单元测试（python3 -m unittest discover -s scripts/dep-guard）。"""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import dead_deps
import dep_guard

TEXT_OUTPUT = """Analyzing dependencies of crates in this directory...
cargo-machete found the following unused dependencies in this directory:
closeclaw-gateway -- ./crates/gateway/Cargo.toml:
\thex
closeclaw -- ./Cargo.toml:
\tdirs
\tfutures

If you believe cargo-machete has detected an unused dependency incorrectly,
you can add the dependency to the list of dependencies to ignore in the
`[package.metadata.cargo-machete]` section of the appropriate Cargo.toml.
For example:

[package.metadata.cargo-machete]
ignored = ["prost"]

Done!
"""

JSON_OUTPUT = [
    {
        "package_name": "closeclaw-gateway",
        "manifest_path": "./crates/gateway/Cargo.toml",
        "unused_deps": ["hex"],
    },
    {
        "package_name": "closeclaw-tools",
        "manifest_path": "./crates/tools/Cargo.toml",
        "unused_deps": ["serde", "dashmap"],
    },
]


def _completed(returncode: int, stdout: str = "", stderr: str = "") -> subprocess.CompletedProcess:
    return subprocess.CompletedProcess(
        args=["cargo", "machete"], returncode=returncode, stdout=stdout, stderr=stderr
    )


class MacheteAvailableTest(unittest.TestCase):
    def test_missing_tool_returns_false(self) -> None:
        with mock.patch.object(
            dead_deps.subprocess, "run", side_effect=FileNotFoundError("cargo")
        ):
            self.assertFalse(dead_deps.machete_available())

    def test_nonzero_exit_returns_false(self) -> None:
        with mock.patch.object(
            dead_deps.subprocess, "run", return_value=_completed(1, stderr="boom")
        ):
            self.assertFalse(dead_deps.machete_available())

    def test_version_ok_returns_true(self) -> None:
        with mock.patch.object(
            dead_deps.subprocess, "run", return_value=_completed(0, stdout="0.9.2")
        ):
            self.assertTrue(dead_deps.machete_available())


class ParseJsonTest(unittest.TestCase):
    def test_parse_normal(self) -> None:
        unused = dead_deps.parse_json_output(json.loads(json.dumps(JSON_OUTPUT)))
        self.assertEqual(
            unused,
            {
                "closeclaw-gateway": ["hex"],
                "closeclaw-tools": ["dashmap", "serde"],
            },
        )

    def test_parse_empty_deps_entry_ignored(self) -> None:
        unused = dead_deps.parse_json_output(
            [{"package_name": "closeclaw-clean", "unused_deps": []}]
        )
        self.assertEqual(unused, {})

    def test_parse_duplicate_names_merged(self) -> None:
        unused = dead_deps.parse_json_output(
            [
                {"package_name": "pkg", "unused_deps": ["b", "a"]},
                {"package_name": "pkg", "unused_deps": ["a", "c"]},
            ]
        )
        self.assertEqual(unused, {"pkg": ["a", "b", "c"]})

    def test_parse_entry_missing_package_name_raises(self) -> None:
        with self.assertRaises(dead_deps.DeadDepsError):
            dead_deps.parse_json_output([{"unused_deps": ["hex"]}])

    def test_parse_non_dict_entry_raises(self) -> None:
        with self.assertRaises(dead_deps.DeadDepsError):
            dead_deps.parse_json_output(["closeclaw-gateway"])


class ParseTextTest(unittest.TestCase):
    def test_parse_normal(self) -> None:
        unused = dead_deps.parse_text_output(TEXT_OUTPUT)
        self.assertEqual(
            unused,
            {"closeclaw-gateway": ["hex"], "closeclaw": ["dirs", "futures"]},
        )

    def test_parse_no_unused(self) -> None:
        text = "Analyzing dependencies of crates in this directory...\nDone!\n"
        self.assertEqual(dead_deps.parse_text_output(text), {})

    def test_parse_trailer_not_treated_as_deps(self) -> None:
        unused = dead_deps.parse_text_output(TEXT_OUTPUT)
        self.assertNotIn("ignored", unused)
        self.assertEqual(unused.get("closeclaw"), ["dirs", "futures"])

    def test_parse_space_indented_deps(self) -> None:
        text = (
            "pkg-a -- ./crates/a/Cargo.toml:\n"
            "    serde\n"
            "    tracing\n"
        )
        self.assertEqual(dead_deps.parse_text_output(text), {"pkg-a": ["serde", "tracing"]})


class CollectUnusedTest(unittest.TestCase):
    def test_prefers_json(self) -> None:
        runs = [
            _completed(1, stdout=json.dumps(JSON_OUTPUT)),
            mock.ANY,
        ]

        def fake_run(repo_root, extra_args):
            self.assertEqual(extra_args, ["--json"])
            return runs.pop(0)

        with mock.patch.object(dead_deps, "_run_machete", side_effect=fake_run):
            data = dead_deps.collect_unused(Path("."))
        self.assertEqual(data.source, "json")
        self.assertEqual(data.unused["closeclaw-gateway"], ["hex"])

    def test_falls_back_to_text_when_json_unsupported(self) -> None:
        with mock.patch.object(
            dead_deps,
            "_run_machete",
            side_effect=[
                _completed(2, stderr="Unrecognized argument: --json"),
                _completed(1, stdout=TEXT_OUTPUT),
            ],
        ) as run_mock:
            data = dead_deps.collect_unused(Path("."))
        self.assertEqual(data.source, "text")
        self.assertEqual(data.unused["closeclaw-gateway"], ["hex"])
        self.assertEqual(run_mock.call_count, 2)

    def test_text_run_failure_raises(self) -> None:
        with mock.patch.object(
            dead_deps,
            "_run_machete",
            side_effect=[
                _completed(2, stderr="Unrecognized argument: --json"),
                _completed(2, stderr="manifest parse error"),
            ],
        ):
            with self.assertRaises(dead_deps.DeadDepsError):
                dead_deps.collect_unused(Path("."))


class UnusedItemsTest(unittest.TestCase):
    def test_format_and_sort(self) -> None:
        items = dead_deps.unused_items(
            {"closeclaw-gateway": ["hex"], "closeclaw": ["futures", "dirs"]}
        )
        self.assertEqual(
            items,
            [
                "closeclaw-gateway: hex",
                "closeclaw: dirs",
                "closeclaw: futures",
            ],
        )


class CheckDeadDepsTest(unittest.TestCase):
    def _write_baseline(self, tmp: str, content: str) -> Path:
        path = Path(tmp) / "dead-deps.txt"
        path.write_text(content, encoding="utf-8")
        return path

    def test_tool_missing_skips(self) -> None:
        with mock.patch.object(dead_deps, "machete_available", return_value=False):
            result = dep_guard.check_dead_deps(Path("."))
        self.assertEqual(result.status, dep_guard.STATUS_SKIP)

    def test_new_item_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            baseline_dir = Path(tmp)
            self._write_baseline(tmp, "closeclaw-gateway: hex\n")
            data = dead_deps.MacheteData(
                unused={"closeclaw-gateway": ["hex", "serde"]}, source="text"
            )
            with mock.patch.object(dead_deps, "machete_available", return_value=True), \
                    mock.patch.object(dead_deps, "collect_unused", return_value=data), \
                    mock.patch.object(dep_guard, "BASELINE_DIR", baseline_dir):
                result = dep_guard.check_dead_deps(Path("."))
        self.assertEqual(result.status, dep_guard.STATUS_FAIL)
        self.assertTrue(any("closeclaw-gateway: serde" in line for line in result.lines))

    def test_eliminated_item_passes_with_info(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            baseline_dir = Path(tmp)
            self._write_baseline(tmp, "closeclaw-gateway: hex\nold-crate: gone\n")
            data = dead_deps.MacheteData(unused={"closeclaw-gateway": ["hex"]}, source="json")
            with mock.patch.object(dead_deps, "machete_available", return_value=True), \
                    mock.patch.object(dead_deps, "collect_unused", return_value=data), \
                    mock.patch.object(dep_guard, "BASELINE_DIR", baseline_dir):
                result = dep_guard.check_dead_deps(Path("."))
        self.assertEqual(result.status, dep_guard.STATUS_PASS)
        self.assertTrue(any("old-crate: gone" in line for line in result.lines))

    def test_machete_error_fails(self) -> None:
        with mock.patch.object(dead_deps, "machete_available", return_value=True), \
                mock.patch.object(
                    dead_deps,
                    "collect_unused",
                    side_effect=dead_deps.DeadDepsError("cargo machete 失败"),
                ):
            result = dep_guard.check_dead_deps(Path("."))
        self.assertEqual(result.status, dep_guard.STATUS_FAIL)


if __name__ == "__main__":
    unittest.main()
