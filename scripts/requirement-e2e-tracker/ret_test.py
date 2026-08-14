#!/usr/bin/env python3
"""
Unit tests for ret.py (Requirement E2E Tracker).

Run: python3 ret_test.py
"""

from __future__ import annotations

import argparse
import atexit
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

# ---------------------------------------------------------------------------
# Patch module-level globals BEFORE importing ret, so the import succeeds
# even outside a git repo.
# ---------------------------------------------------------------------------

_FAKE_REPO = tempfile.mkdtemp(prefix="ret_test_repo_")
atexit.register(shutil.rmtree, _FAKE_REPO, ignore_errors=True)

_original_check_output = __import__("subprocess").check_output


def _patched_check_output(cmd, **kwargs):
    if "rev-parse" in cmd and "--show-toplevel" in cmd:
        return _FAKE_REPO + "\n"
    return _original_check_output(cmd, **kwargs)


with mock.patch("subprocess.check_output", side_effect=_patched_check_output):
    import ret


# Now fix module globals to point inside our fake repo
ret.REPO_ROOT = Path(_FAKE_REPO)
ret.RECORDS_FILE = Path(_FAKE_REPO) / "records.json"
ret.REQUIREMENTS_DIR = Path(_FAKE_REPO) / "docs" / "requirements"
ret.DESIGN_DOC_DIR = Path(_FAKE_REPO) / "docs" / "design"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_args(**kwargs) -> argparse.Namespace:
    return argparse.Namespace(**kwargs)


def _write_json(path: Path, data):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f)


def _read_json(path: Path):
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


# ---------------------------------------------------------------------------
# Tests – _load_records / _save_records
# ---------------------------------------------------------------------------


class TestLoadSaveRecords(unittest.TestCase):
    def setUp(self):
        self._orig_records = ret.RECORDS_FILE
        self._tmp = tempfile.NamedTemporaryFile(
            suffix=".json", delete=False, mode="w"
        )
        self._tmp.close()
        ret.RECORDS_FILE = Path(self._tmp.name)

    def tearDown(self):
        ret.RECORDS_FILE = self._orig_records
        os.unlink(self._tmp.name)

    def test_load_returns_empty_list_when_no_file(self):
        ret.RECORDS_FILE = Path("/nonexistent/path.json")
        self.assertEqual(ret._load_records(), [])

    def test_load_adds_blocked_reason_default(self):
        data = [{"module": "agent", "commit": "abc"}]
        _write_json(Path(self._tmp.name), data)
        result = ret._load_records()
        expected = [{"module": "agent", "commit": "abc", "blocked_reason": ""}]
        self.assertEqual(result, expected)

    def test_save_sorts_by_module(self):
        records = [
            {"module": "z", "commit": "1"},
            {"module": "a", "commit": "2"},
        ]
        ret._save_records(records)
        result = _read_json(Path(self._tmp.name))
        self.assertEqual([r["module"] for r in result], ["a", "z"])

    def test_save_roundtrip(self):
        records = [
            {"module": "c", "commit": "c1"},
            {"module": "a", "commit": "a1"},
        ]
        ret._save_records(records)
        loaded = ret._load_records()
        expected = [
            {"module": "a", "commit": "a1", "blocked_reason": ""},
            {"module": "c", "commit": "c1", "blocked_reason": ""},
        ]
        self.assertEqual(loaded, expected)


# ---------------------------------------------------------------------------
# Tests – _discover_modules / _module_docs
# ---------------------------------------------------------------------------


class TestModuleDiscovery(unittest.TestCase):
    def setUp(self):
        self._orig_repo = ret.REPO_ROOT
        self._orig_req = ret.REQUIREMENTS_DIR
        self._orig_design = ret.DESIGN_DOC_DIR
        self._tmpdir = tempfile.mkdtemp(prefix="ret_discover_")
        self._fake_repo = Path(self._tmpdir) / "repo"
        self._fake_repo.mkdir()
        ret.REPO_ROOT = self._fake_repo
        ret.REQUIREMENTS_DIR = self._fake_repo / "docs" / "requirements"
        ret.DESIGN_DOC_DIR = self._fake_repo / "docs" / "design"

    def tearDown(self):
        ret.REPO_ROOT = self._orig_repo
        ret.REQUIREMENTS_DIR = self._orig_req
        ret.DESIGN_DOC_DIR = self._orig_design
        shutil.rmtree(self._tmpdir)

    def test_discover_modules_excludes_meta(self):
        ret.REQUIREMENTS_DIR.mkdir(parents=True)
        for name in ["agent.md", "session.md", "README.md", "STANDARDS.md"]:
            (ret.REQUIREMENTS_DIR / name).write_text(f"# {name}")
        modules = ret._discover_modules()
        self.assertEqual(modules, ["agent", "session"])

    def test_discover_modules_ignores_subdirs(self):
        ret.REQUIREMENTS_DIR.mkdir(parents=True)
        (ret.REQUIREMENTS_DIR / "agent.md").write_text("# agent")
        sub = ret.REQUIREMENTS_DIR / "im_adapter"
        sub.mkdir()
        (sub / "feishu.md").write_text("# feishu")
        modules = ret._discover_modules()
        self.assertEqual(modules, ["agent"])

    def test_discover_modules_no_dir(self):
        modules = ret._discover_modules()
        self.assertEqual(modules, [])

    def test_module_docs_requirement_only(self):
        ret.REQUIREMENTS_DIR.mkdir(parents=True)
        (ret.REQUIREMENTS_DIR / "agent.md").write_text("# agent")
        docs = [str(d) for d in ret._module_docs("agent")]
        self.assertEqual(docs, ["docs/requirements/agent.md"])

    def test_module_docs_with_design_dir(self):
        ret.REQUIREMENTS_DIR.mkdir(parents=True)
        (ret.REQUIREMENTS_DIR / "agent.md").write_text("# agent")
        design = ret.DESIGN_DOC_DIR / "agent"
        design.mkdir(parents=True)
        (design / "config.md").write_text("# config")
        sub = design / "sub"
        sub.mkdir()
        (sub / "nested.md").write_text("# nested")

        docs = sorted(str(d) for d in ret._module_docs("agent"))
        self.assertIn("docs/requirements/agent.md", docs)
        self.assertIn("docs/design/agent/config.md", docs)
        self.assertIn("docs/design/agent/sub/nested.md", docs)
        self.assertEqual(len(docs), 3)

    def test_module_docs_unknown_module_empty(self):
        self.assertEqual(ret._module_docs("nope"), [])


# ---------------------------------------------------------------------------
# Tests – cmd_finished
# ---------------------------------------------------------------------------


class TestCmdFinished(unittest.TestCase):
    def setUp(self):
        self._orig_repo = ret.REPO_ROOT
        self._orig_records = ret.RECORDS_FILE
        self._orig_req = ret.REQUIREMENTS_DIR
        self._orig_design = ret.DESIGN_DOC_DIR
        self._tmpdir = tempfile.mkdtemp(prefix="ret_finished_")
        self._fake_repo = Path(self._tmpdir) / "repo"
        self._fake_repo.mkdir()
        ret.REPO_ROOT = self._fake_repo
        ret.RECORDS_FILE = self._fake_repo / "records.json"
        ret.REQUIREMENTS_DIR = self._fake_repo / "docs" / "requirements"
        ret.DESIGN_DOC_DIR = self._fake_repo / "docs" / "design"

    def tearDown(self):
        ret.REPO_ROOT = self._orig_repo
        ret.RECORDS_FILE = self._orig_records
        ret.REQUIREMENTS_DIR = self._orig_req
        ret.DESIGN_DOC_DIR = self._orig_design
        shutil.rmtree(self._tmpdir)

    def _create_module(self, module: str, with_design: bool = False):
        ret.REQUIREMENTS_DIR.mkdir(parents=True, exist_ok=True)
        (ret.REQUIREMENTS_DIR / f"{module}.md").write_text(f"# {module}")
        if with_design:
            design = ret.DESIGN_DOC_DIR / module
            design.mkdir(parents=True)
            (design / "config.md").write_text("# config")

    def test_finished_unknown_module_errors(self):
        args = _make_args(module="nope")
        import io, sys
        old_stderr = sys.stderr
        sys.stderr = buf = io.StringIO()
        try:
            rc = ret.cmd_finished(args)
        finally:
            sys.stderr = old_stderr
        self.assertEqual(rc, 1)
        self.assertIn("nope", buf.getvalue())

    def test_finished_merge_base_failure(self):
        self._create_module("agent")
        args = _make_args(module="agent")
        import io, sys
        old_stderr = sys.stderr
        sys.stderr = buf = io.StringIO()
        try:
            with mock.patch.object(ret, "_merge_base_commit", return_value=None):
                rc = ret.cmd_finished(args)
        finally:
            sys.stderr = old_stderr
        self.assertEqual(rc, 1)
        self.assertIn("merge-base", buf.getvalue())

    def test_finished_happy_path(self):
        self._create_module("agent", with_design=True)
        args = _make_args(module="agent")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            with mock.patch.object(ret, "_merge_base_commit", return_value="merge001"), \
                 mock.patch.object(ret, "_commit_committer_date", return_value="2025-03-01T00:00:00+00:00"), \
                 mock.patch.object(ret, "_now_iso", return_value="2025-06-12T00:00:00+08:00"):
                rc = ret.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        self.assertIn("module 'agent'", buf.getvalue())
        records = _read_json(ret.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["module"], "agent")
        self.assertEqual(records[0]["commit"], "merge001")
        self.assertEqual(records[0]["comment"], "")
        self.assertEqual(records[0]["blocked_reason"], "")

    def test_finished_idempotent(self):
        self._create_module("agent")
        args = _make_args(module="agent")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            with mock.patch.object(ret, "_merge_base_commit", return_value="v1"), \
                 mock.patch.object(ret, "_commit_committer_date", return_value="2025-01-01T00:00:00+00:00"), \
                 mock.patch.object(ret, "_now_iso", return_value="2025-01-01T00:00:00+08:00"):
                ret.cmd_finished(args)
            with mock.patch.object(ret, "_merge_base_commit", return_value="v2"), \
                 mock.patch.object(ret, "_commit_committer_date", return_value="2025-02-01T00:00:00+00:00"), \
                 mock.patch.object(ret, "_now_iso", return_value="2025-02-01T00:00:00+08:00"):
                ret.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        records = _read_json(ret.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["commit"], "v2")

    def test_finished_clears_comment_and_blocked(self):
        self._create_module("agent")
        _write_json(ret.RECORDS_FILE, [{
            "module": "agent",
            "commit": "old",
            "commit_time": "2025-01-01T00:00:00+00:00",
            "confirmed_time": "2025-01-01T00:00:00+08:00",
            "comment": "stale comment",
            "blocked_reason": "blocked before",
        }])
        args = _make_args(module="agent")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            with mock.patch.object(ret, "_merge_base_commit", return_value="new"), \
                 mock.patch.object(ret, "_commit_committer_date", return_value="2025-07-01T00:00:00+00:00"), \
                 mock.patch.object(ret, "_now_iso", return_value="2025-07-01T00:00:00+08:00"):
                ret.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        records = _read_json(ret.RECORDS_FILE)
        self.assertEqual(records[0]["comment"], "")
        self.assertEqual(records[0]["blocked_reason"], "")
        self.assertEqual(records[0]["commit"], "new")


# ---------------------------------------------------------------------------
# Tests – cmd_comment / cmd_blocked
# ---------------------------------------------------------------------------


class TestCmdCommentBlocked(unittest.TestCase):
    def setUp(self):
        self._orig_repo = ret.REPO_ROOT
        self._orig_records = ret.RECORDS_FILE
        self._tmpdir = tempfile.mkdtemp(prefix="ret_comment_")
        self._fake_repo = Path(self._tmpdir) / "repo"
        self._fake_repo.mkdir()
        ret.REPO_ROOT = self._fake_repo
        ret.RECORDS_FILE = self._fake_repo / "records.json"

    def tearDown(self):
        ret.REPO_ROOT = self._orig_repo
        ret.RECORDS_FILE = self._orig_records
        shutil.rmtree(self._tmpdir)

    def test_comment_updates_existing(self):
        _write_json(ret.RECORDS_FILE, [
            {"module": "agent", "commit": "aaa111", "comment": ""}
        ])
        args = _make_args(module="agent", text="needs review")
        rc = ret.cmd_comment(args)
        self.assertEqual(rc, 0)
        records = _read_json(ret.RECORDS_FILE)
        self.assertEqual(records[0]["comment"], "needs review")

    def test_comment_creates_record(self):
        _write_json(ret.RECORDS_FILE, [])
        args = _make_args(module="agent", text="first")
        rc = ret.cmd_comment(args)
        self.assertEqual(rc, 0)
        records = _read_json(ret.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["module"], "agent")
        self.assertEqual(records[0]["commit"], "")
        self.assertEqual(records[0]["comment"], "first")

    def test_blocked_creates_record(self):
        _write_json(ret.RECORDS_FILE, [])
        args = _make_args(module="agent", reason="waiting")
        rc = ret.cmd_blocked(args)
        self.assertEqual(rc, 0)
        records = _read_json(ret.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["module"], "agent")
        self.assertEqual(records[0]["blocked_reason"], "waiting")
        self.assertEqual(records[0]["commit"], "")

    def test_blocked_updates_existing(self):
        _write_json(ret.RECORDS_FILE, [{
            "module": "agent",
            "commit": "aaa111",
            "comment": "keep me",
            "blocked_reason": "",
        }])
        args = _make_args(module="agent", reason="new reason")
        rc = ret.cmd_blocked(args)
        self.assertEqual(rc, 0)
        records = _read_json(ret.RECORDS_FILE)
        self.assertEqual(records[0]["blocked_reason"], "new reason")
        self.assertEqual(records[0]["comment"], "keep me")
        self.assertEqual(records[0]["commit"], "aaa111")


# ---------------------------------------------------------------------------
# Tests – cmd_check
# ---------------------------------------------------------------------------


class TestCmdCheck(unittest.TestCase):
    def setUp(self):
        self._orig_repo = ret.REPO_ROOT
        self._orig_records = ret.RECORDS_FILE
        self._orig_req = ret.REQUIREMENTS_DIR
        self._orig_design = ret.DESIGN_DOC_DIR
        self._tmpdir = tempfile.mkdtemp(prefix="ret_check_")
        self._fake_repo = Path(self._tmpdir) / "repo"
        self._fake_repo.mkdir()
        ret.REPO_ROOT = self._fake_repo
        ret.RECORDS_FILE = self._fake_repo / "records.json"
        ret.REQUIREMENTS_DIR = self._fake_repo / "docs" / "requirements"
        ret.DESIGN_DOC_DIR = self._fake_repo / "docs" / "design"

    def tearDown(self):
        ret.REPO_ROOT = self._orig_repo
        ret.RECORDS_FILE = self._orig_records
        ret.REQUIREMENTS_DIR = self._orig_req
        ret.DESIGN_DOC_DIR = self._orig_design
        shutil.rmtree(self._tmpdir)

    def _create_module(self, module: str, with_design: bool = False):
        ret.REQUIREMENTS_DIR.mkdir(parents=True, exist_ok=True)
        (ret.REQUIREMENTS_DIR / f"{module}.md").write_text(f"# {module}")
        if with_design:
            design = ret.DESIGN_DOC_DIR / module
            design.mkdir(parents=True)
            (design / "config.md").write_text("# config")

    def _write_records(self, records):
        _write_json(ret.RECORDS_FILE, records)

    def _capture(self, fn, *args):
        import io, sys
        old_stdout = sys.stdout
        buf = io.StringIO()
        sys.stdout = buf
        try:
            rc = fn(*args)
        finally:
            sys.stdout = old_stdout
        return rc, buf.getvalue()

    def test_no_requirements_dir(self):
        args = _make_args()
        rc, out = self._capture(ret.cmd_check, args)
        self.assertEqual(rc, 0)
        self.assertEqual(out.strip(), "")

    def test_all_untracked(self):
        self._create_module("agent")
        self._create_module("session")
        self._write_records([])
        args = _make_args()
        rc, out = self._capture(ret.cmd_check, args)
        self.assertEqual(rc, 0)
        lines = [l.strip() for l in out.strip().splitlines()]
        self.assertEqual(sorted(lines), ["agent", "session"])

    def test_no_change_not_reported(self):
        self._create_module("agent")
        self._write_records([{"module": "agent", "commit": "aaa111"}])
        args = _make_args()
        with mock.patch.object(ret, "_run") as mock_run:
            mock_run.return_value = subprocess.CompletedProcess(
                args=[], returncode=0, stdout="", stderr=""
            )
            rc, out = self._capture(ret.cmd_check, args)
        self.assertEqual(rc, 0)
        self.assertEqual(out.strip(), "")

    def test_changed_reported(self):
        self._create_module("agent")
        self._create_module("session")
        self._write_records([
            {"module": "agent", "commit": "aaa111"},
            {"module": "session", "commit": "bbb222"},
        ])
        args = _make_args()

        def fake_run(cmd, **kwargs):
            if "agent" in str(cmd):
                return subprocess.CompletedProcess(
                    args=cmd, returncode=0, stdout="", stderr=""
                )
            return subprocess.CompletedProcess(
                args=cmd, returncode=1, stdout="", stderr=""
            )

        with mock.patch.object(ret, "_run", side_effect=fake_run):
            rc, out = self._capture(ret.cmd_check, args)

        self.assertEqual(rc, 0)
        lines = [l.strip() for l in out.strip().splitlines()]
        self.assertEqual(lines, ["session"])

    def test_changed_with_comment_output(self):
        self._create_module("agent")
        self._write_records([
            {"module": "agent", "commit": "aaa111", "comment": "important"}
        ])
        args = _make_args()
        with mock.patch.object(ret, "_run") as mock_run:
            mock_run.return_value = subprocess.CompletedProcess(
                args=[], returncode=1, stdout="", stderr=""
            )
            rc, out = self._capture(ret.cmd_check, args)
        self.assertEqual(rc, 0)
        self.assertEqual(out.strip(), "agent\timportant")

    def test_empty_commit_treated_as_changed(self):
        self._create_module("agent")
        self._write_records([{"module": "agent", "commit": "", "comment": ""}])
        args = _make_args()
        with mock.patch.object(ret, "_run") as mock_run:
            rc, out = self._capture(ret.cmd_check, args)
        self.assertEqual(rc, 0)
        mock_run.assert_not_called()
        self.assertIn("agent", out)

    def test_diff_command_includes_requirement_and_design(self):
        self._create_module("agent", with_design=True)
        self._write_records([{"module": "agent", "commit": "aaa111"}])
        args = _make_args()
        seen = {}

        def fake_run(cmd, **kwargs):
            seen["cmd"] = list(cmd)
            return subprocess.CompletedProcess(
                args=cmd, returncode=0, stdout="", stderr=""
            )

        with mock.patch.object(ret, "_run", side_effect=fake_run):
            rc, _ = self._capture(ret.cmd_check, args)
        self.assertEqual(rc, 0)
        cmd_str = " ".join(seen["cmd"])
        self.assertIn("docs/requirements/agent.md", cmd_str)
        self.assertIn("docs/design/agent/config.md", cmd_str)

    def test_blocked_unchanged_not_reported(self):
        self._create_module("agent")
        self._write_records([
            {"module": "agent", "commit": "aaa111", "blocked_reason": "waiting"}
        ])
        args = _make_args()
        with mock.patch.object(ret, "_run") as mock_run:
            mock_run.return_value = subprocess.CompletedProcess(
                args=[], returncode=0, stdout="", stderr=""
            )
            rc, out = self._capture(ret.cmd_check, args)
        self.assertEqual(rc, 0)
        self.assertEqual(out.strip(), "")

    def test_blocked_changed_auto_unblocks(self):
        self._create_module("agent")
        self._write_records([
            {"module": "agent", "commit": "aaa111", "blocked_reason": "waiting"}
        ])
        args = _make_args()

        def fake_run(cmd, **kwargs):
            return subprocess.CompletedProcess(
                args=cmd, returncode=1, stdout="", stderr=""
            )

        with mock.patch.object(ret, "_run", side_effect=fake_run), \
             mock.patch.object(ret, "_merge_base_commit", return_value="newcommit"), \
             mock.patch.object(ret, "_commit_committer_date", return_value="2025-07-01T00:00:00+00:00"), \
             mock.patch.object(ret, "_now_iso", return_value="2025-07-01T00:00:00+08:00"):
            rc, out = self._capture(ret.cmd_check, args)

        self.assertEqual(rc, 0)
        self.assertEqual(out.strip(), "agent")
        records = _read_json(ret.RECORDS_FILE)
        self.assertEqual(records[0]["blocked_reason"], "")
        self.assertEqual(records[0]["commit"], "newcommit")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


if __name__ == "__main__":
    unittest.main()
