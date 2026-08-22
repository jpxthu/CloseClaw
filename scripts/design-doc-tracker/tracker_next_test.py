#!/usr/bin/env python3
"""
Unit tests for ../tracker-next.py (Tracker-Next).

Run: python3 tracker_next_test.py   (from this directory)
     python3 scripts/design-doc-tracker/tracker_next_test.py   (repo root)

Covers the 4 status branches + the blocked-silently-skipped scenario,
using temp records.json / design / requirements fixtures.
"""

from __future__ import annotations

import atexit
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

# ---------------------------------------------------------------------------
# Patch module-level globals BEFORE importing tracker_next, so the import
# (which loads ddt/ret, which resolve REPO_ROOT via git rev-parse)
# succeeds even outside a git repo.
# ---------------------------------------------------------------------------

_FAKE_REPO = tempfile.mkdtemp(prefix="tracker_next_test_repo_")
atexit.register(shutil.rmtree, _FAKE_REPO, ignore_errors=True)

_original_check_output = __import__("subprocess").check_output


def _patched_check_output(cmd, **kwargs):
    if "rev-parse" in cmd and "--show-toplevel" in cmd:
        return _FAKE_REPO + "\n"
    return _original_check_output(cmd, **kwargs)


with mock.patch("subprocess.check_output", side_effect=_patched_check_output):
    import importlib.util
    import sys

    _SCRIPTS_DIR = Path(__file__).resolve().parent.parent
    if str(_SCRIPTS_DIR) not in sys.path:
        sys.path.insert(0, str(_SCRIPTS_DIR))

    _spec = importlib.util.spec_from_file_location(
        "tracker_next", _SCRIPTS_DIR / "tracker-next.py"
    )
    tracker_next = importlib.util.module_from_spec(_spec)
    sys.modules["tracker_next"] = tracker_next
    _spec.loader.exec_module(tracker_next)

    ddt = tracker_next.ddt
    ret = tracker_next.ret

# Point both trackers' globals at our fake repo
ddt.REPO_ROOT = Path(_FAKE_REPO)
ddt.RECORDS_FILE = Path(_FAKE_REPO) / "ddt_records.json"
ddt.DESIGN_DOC_DIR = Path(_FAKE_REPO) / "docs" / "design"
ret.REPO_ROOT = Path(_FAKE_REPO)
ret.RECORDS_FILE = Path(_FAKE_REPO) / "ret_records.json"
ret.REQUIREMENTS_DIR = Path(_FAKE_REPO) / "docs" / "requirements"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _write_json(path: Path, data):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f)


def _read_json(path: Path):
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def _record(path="", module="", commit="c0", comment="", blocked=""):
    """Build a design-doc style record (path mode) by default."""
    return {
        "path": path,
        "module": module,
        "commit": commit,
        "commit_time": "",
        "confirmed_time": "",
        "comment": comment,
        "blocked_reason": blocked,
    }


class _Fixture(unittest.TestCase):
    """Shared fixture: per-test temp dirs wired into ddt/ret globals."""

    def setUp(self):
        self._tmpdir = tempfile.mkdtemp(prefix="tracker_next_case_")
        self._repo = Path(self._tmpdir) / "repo"
        self._repo.mkdir()
        self._saved = (
            ddt.REPO_ROOT, ddt.RECORDS_FILE, ddt.DESIGN_DOC_DIR,
            ret.REPO_ROOT, ret.RECORDS_FILE, ret.REQUIREMENTS_DIR,
        )
        ddt.REPO_ROOT = self._repo
        ddt.RECORDS_FILE = self._repo / "ddt_records.json"
        ddt.DESIGN_DOC_DIR = self._repo / "docs" / "design"
        ret.REPO_ROOT = self._repo
        ret.RECORDS_FILE = self._repo / "ret_records.json"
        ret.REQUIREMENTS_DIR = self._repo / "docs" / "requirements"

    def tearDown(self):
        (
            ddt.REPO_ROOT, ddt.RECORDS_FILE, ddt.DESIGN_DOC_DIR,
            ret.REPO_ROOT, ret.RECORDS_FILE, ret.REQUIREMENTS_DIR,
        ) = self._saved
        shutil.rmtree(self._tmpdir)

    def _create_design_docs(self, names):
        ddt.DESIGN_DOC_DIR.mkdir(parents=True, exist_ok=True)
        for name in names:
            (ddt.DESIGN_DOC_DIR / name).write_text(f"# {name}")

    def _create_requirement_docs(self, modules):
        ret.REQUIREMENTS_DIR.mkdir(parents=True, exist_ok=True)
        for m in modules:
            (ret.REQUIREMENTS_DIR / f"{m}.md").write_text(f"# {m}")

    def _no_change_run(self):
        """A ddt/ret _run fake: every git diff --quiet says 'unchanged'."""
        return subprocess.CompletedProcess(args=[], returncode=0, stdout="", stderr="")

    def _changed_run(self):
        return subprocess.CompletedProcess(args=[], returncode=1, stdout="", stderr="")


# ---------------------------------------------------------------------------
# Tests – 4 status branches
# ---------------------------------------------------------------------------


class TestStatusDesign(_Fixture):
    def test_changed_design_doc_reported(self):
        """A changed design doc → status=design, doc listed with comment."""
        self._create_design_docs(["auth.md"])
        _write_json(ddt.RECORDS_FILE, [
            _record(path="docs/design/auth.md", commit="old", comment="rework needed"),
        ])
        with mock.patch.object(ddt, "_run", return_value=self._changed_run()):
            result = tracker_next.cmd_next()
        self.assertEqual(result["status"], "design")
        self.assertEqual(
            result["design_docs"],
            [{"path": "docs/design/auth.md", "comment": "rework needed"}],
        )
        self.assertEqual(result["blocked_docs"], [])
        self.assertEqual(result["requirement_modules"], [])

    def test_untracked_design_doc_reported(self):
        """No record at all → doc is reported as changed."""
        self._create_design_docs(["brand_new.md"])
        _write_json(ddt.RECORDS_FILE, [])
        with mock.patch.object(ddt, "_run", return_value=self._changed_run()):
            result = tracker_next.cmd_next()
        self.assertEqual(result["status"], "design")
        self.assertEqual(
            result["design_docs"],
            [{"path": "docs/design/brand_new.md", "comment": ""}],
        )

    def test_design_takes_priority_even_with_blocked_and_requirements(self):
        """Design changes win over blocked records and requirement changes."""
        self._create_design_docs(["a.md"])
        self._create_requirement_docs(["session"])
        _write_json(ddt.RECORDS_FILE, [
            _record(path="docs/design/a.md", commit="old"),
            _record(path="docs/design/gone.md", commit="x", blocked="dep missing"),
        ])
        _write_json(ret.RECORDS_FILE, [])
        with mock.patch.object(ddt, "_run", return_value=self._changed_run()):
            result = tracker_next.cmd_next()
        self.assertEqual(result["status"], "design")


class TestStatusBlockedWaiting(_Fixture):
    def test_blocked_unchanged_silently_skipped_by_check_but_gated_here(self):
        """THE key scenario: check output empty, yet a blocked record remains.

        ddt check silently skips blocked docs that have NOT changed, so an
        empty check output does not mean 'all confirmed'.  tracker-next must
        surface it as blocked-waiting.
        """
        self._create_design_docs(["auth.md"])
        _write_json(ddt.RECORDS_FILE, [
            _record(path="docs/design/auth.md", commit="c0",
                    blocked="waiting on external API"),
        ])
        # git diff --quiet says unchanged → ddt check output would be empty
        with mock.patch.object(ddt, "_run", return_value=self._no_change_run()):
            result = tracker_next.cmd_next()
        self.assertEqual(result["status"], "blocked-waiting")
        self.assertEqual(
            result["blocked_docs"],
            [{"path": "docs/design/auth.md", "reason": "waiting on external API"}],
        )
        self.assertEqual(result["design_docs"], [])
        self.assertEqual(result["requirement_modules"], [])

    def test_blocked_record_for_deleted_doc_still_gates(self):
        """A blocked record whose doc no longer exists still blocks."""
        _write_json(ddt.RECORDS_FILE, [
            _record(path="docs/design/removed.md", commit="c0", blocked="gone"),
        ])
        result = tracker_next.cmd_next()
        self.assertEqual(result["status"], "blocked-waiting")
        self.assertEqual(
            result["blocked_docs"],
            [{"path": "docs/design/removed.md", "reason": "gone"}],
        )

    def test_empty_commit_records_do_not_block(self):
        """Records with empty blocked_reason never gate (even empty commit)."""
        self._create_design_docs(["ok.md"])
        _write_json(ddt.RECORDS_FILE, [
            _record(path="docs/design/ok.md", commit="c0"),
        ])
        _write_json(ret.RECORDS_FILE, [])
        self._create_requirement_docs([])
        with mock.patch.object(ddt, "_run", return_value=self._no_change_run()):
            result = tracker_next.cmd_next()
        # no design change, no blocked, no requirement change → empty
        self.assertEqual(result["status"], "empty")


class TestStatusRequirement(_Fixture):
    def test_requirement_change_reported_after_gate_passes(self):
        """Design all confirmed + no blocked → requirement changes reported."""
        self._create_design_docs(["auth.md"])
        self._create_requirement_docs(["session", "llm"])
        _write_json(ddt.RECORDS_FILE, [
            _record(path="docs/design/auth.md", commit="c0"),
        ])
        _write_json(ret.RECORDS_FILE, [
            _record(module="session", commit="old", comment="rediscovery needed"),
        ])

        def fake_ddt_run(cmd, **kwargs):
            return self._no_change_run()

        def fake_ret_run(cmd, **kwargs):
            # session changed, llm changed
            return self._changed_run()

        with mock.patch.object(ddt, "_run", side_effect=fake_ddt_run), \
             mock.patch.object(ret, "_run", side_effect=fake_ret_run):
            result = tracker_next.cmd_next()
        self.assertEqual(result["status"], "requirement")
        modules = {m["module"]: m["comment"] for m in result["requirement_modules"]}
        self.assertEqual(modules.get("session"), "rediscovery needed")
        self.assertIn("llm", modules)

    def test_untracked_requirement_module_reported(self):
        """Module with no record → reported at requirement stage."""
        self._create_design_docs(["auth.md"])
        self._create_requirement_docs(["fresh"])
        _write_json(ddt.RECORDS_FILE, [
            _record(path="docs/design/auth.md", commit="c0"),
        ])
        _write_json(ret.RECORDS_FILE, [])
        with mock.patch.object(ddt, "_run", return_value=self._no_change_run()):
            result = tracker_next.cmd_next()
        self.assertEqual(result["status"], "requirement")
        self.assertEqual(
            result["requirement_modules"],
            [{"module": "fresh", "comment": ""}],
        )


class TestStatusEmpty(_Fixture):
    def test_all_confirmed_and_no_requirements_changed(self):
        """Everything confirmed, nothing blocked, no requirement drift → empty."""
        self._create_design_docs(["auth.md"])
        self._create_requirement_docs(["session"])
        _write_json(ddt.RECORDS_FILE, [
            _record(path="docs/design/auth.md", commit="c0"),
        ])
        _write_json(ret.RECORDS_FILE, [
            _record(module="session", commit="c0"),
        ])
        with mock.patch.object(ddt, "_run", return_value=self._no_change_run()), \
             mock.patch.object(ret, "_run", return_value=self._no_change_run()):
            result = tracker_next.cmd_next()
        self.assertEqual(result["status"], "empty")
        self.assertEqual(result["design_docs"], [])
        self.assertEqual(result["blocked_docs"], [])
        self.assertEqual(result["requirement_modules"], [])

    def test_no_docs_at_all(self):
        """No design dir, no requirements dir, no records → empty."""
        _write_json(ddt.RECORDS_FILE, [])
        _write_json(ret.RECORDS_FILE, [])
        result = tracker_next.cmd_next()
        self.assertEqual(result["status"], "empty")


# ---------------------------------------------------------------------------
# Tests – read-only guarantee
# ---------------------------------------------------------------------------


class TestReadOnly(_Fixture):
    def test_records_files_never_written(self):
        """cmd_next must not write any records.json (no auto-unblock persistence)."""
        design_records = [
            # blocked + changed would normally trigger auto-unblock in ddt check
            _record(path="docs/design/auth.md", commit="c0", blocked="will unblock"),
        ]
        self._create_design_docs(["auth.md"])
        _write_json(ddt.RECORDS_FILE, design_records)
        _write_json(ret.RECORDS_FILE, [])

        with mock.patch.object(ddt, "_run", return_value=self._changed_run()):
            result = tracker_next.cmd_next()

        self.assertEqual(result["status"], "design")
        # file content unchanged, blocked_reason preserved on disk
        self.assertEqual(_read_json(ddt.RECORDS_FILE), design_records)

    def test_no_temp_or_lock_files_created(self):
        """Running cmd_next leaves the fake repo without stray files."""
        self._create_design_docs(["a.md"])
        _write_json(ddt.RECORDS_FILE, [_record(path="docs/design/a.md", commit="c0")])
        _write_json(ret.RECORDS_FILE, [])
        before = sorted(p.name for p in self._repo.rglob("*"))
        with mock.patch.object(ddt, "_run", return_value=self._no_change_run()):
            tracker_next.cmd_next()
        after = sorted(p.name for p in self._repo.rglob("*"))
        self.assertEqual(before, after)


# ---------------------------------------------------------------------------
# Tests – CLI smoke
# ---------------------------------------------------------------------------


class TestCli(unittest.TestCase):
    def test_next_command_outputs_single_json_object(self):
        """`tracker-next.py next` prints exactly one JSON object, rc=0."""
        repo = Path(
            subprocess.check_output(
                ["git", "rev-parse", "--show-toplevel"],
                cwd=Path(__file__).resolve().parent,
                text=True,
            ).strip()
        )
        proc = subprocess.run(
            [sys.executable, str(repo / "scripts" / "tracker-next.py"), "next"],
            capture_output=True, text=True, timeout=120,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        payload = json.loads(proc.stdout)
        self.assertIsInstance(payload, dict)
        self.assertIn(payload["status"],
                      {"design", "blocked-waiting", "requirement", "empty"})
        self.assertIn("design_docs", payload)
        self.assertIn("blocked_docs", payload)
        self.assertIn("requirement_modules", payload)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    unittest.main()
