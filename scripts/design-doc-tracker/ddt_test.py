#!/usr/bin/env python3
"""
Unit tests for ddt.py (Design Doc Tracker).

Run: python3 ddt_test.py
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
# Patch module-level globals BEFORE importing ddt, so the import succeeds
# even outside a git repo.
# ---------------------------------------------------------------------------

# We need to mock subprocess.check_output so that the module-level
# ``REPO_ROOT = Path(subprocess.check_output(...))`` resolves.
_FAKE_REPO = tempfile.mkdtemp(prefix="ddt_test_repo_")
atexit.register(shutil.rmtree, _FAKE_REPO, ignore_errors=True)

_original_check_output = __import__("subprocess").check_output


def _patched_check_output(cmd, **kwargs):
    if "rev-parse" in cmd and "--show-toplevel" in cmd:
        return _FAKE_REPO + "\n"
    return _original_check_output(cmd, **kwargs)


with mock.patch("subprocess.check_output", side_effect=_patched_check_output):
    import ddt


# Now fix RECORDS_FILE to point inside our fake repo
ddt.REPO_ROOT = Path(_FAKE_REPO)
ddt.RECORDS_FILE = Path(_FAKE_REPO) / "records.json"
ddt.DESIGN_DOC_DIR = Path(_FAKE_REPO) / "docs" / "design"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_args(**kwargs) -> argparse.Namespace:
    """Build a minimal Namespace for cmd_finished / cmd_check / cmd_comment."""
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
    """Tests for _load_records and _save_records."""

    def setUp(self):
        self._orig_records = ddt.RECORDS_FILE
        # Use a fresh temp file for each test
        self._tmp = tempfile.NamedTemporaryFile(
            suffix=".json", delete=False, mode="w"
        )
        self._tmp.close()
        ddt.RECORDS_FILE = Path(self._tmp.name)

    def tearDown(self):
        ddt.RECORDS_FILE = self._orig_records
        os.unlink(self._tmp.name)

    # -- _load_records -----------------------------------------------------

    def test_load_returns_empty_list_when_no_file(self):
        """Non-existent file → []"""
        ddt.RECORDS_FILE = Path("/nonexistent/path.json")
        self.assertEqual(ddt._load_records(), [])

    def test_load_returns_list_from_existing_file(self):
        """Existing file → parsed list."""
        data = [{"path": "a.md", "commit": "abc"}]
        _write_json(Path(self._tmp.name), data)
        result = ddt._load_records()
        self.assertEqual(result, data)

    def test_load_handles_empty_json_array(self):
        """Empty JSON array → []."""
        _write_json(Path(self._tmp.name), [])
        self.assertEqual(ddt._load_records(), [])

    # -- _save_records -----------------------------------------------------

    def test_save_creates_file(self):
        """_save_records should create the file."""
        records = [{"path": "b.md", "commit": "1"}, {"path": "a.md", "commit": "2"}]
        ddt._save_records(records)
        result = _read_json(Path(self._tmp.name))
        self.assertEqual(len(result), 2)

    def test_save_sorts_by_path(self):
        """Records must be sorted by 'path' after saving."""
        records = [
            {"path": "z.md", "commit": "1"},
            {"path": "a.md", "commit": "2"},
            {"path": "m.md", "commit": "3"},
        ]
        ddt._save_records(records)
        result = _read_json(Path(self._tmp.name))
        paths = [r["path"] for r in result]
        self.assertEqual(paths, sorted(paths))

    def test_save_roundtrip(self):
        """load → save → load should be idempotent."""
        records = [
            {"path": "c.md", "commit": "c1"},
            {"path": "a.md", "commit": "a1"},
        ]
        ddt._save_records(records)
        loaded = ddt._load_records()
        self.assertEqual(loaded, records)  # already sorted
        ddt._save_records(loaded)
        loaded2 = ddt._load_records()
        self.assertEqual(loaded2, records)

    def test_save_overwrites_previous_content(self):
        """Calling save twice replaces old content entirely."""
        _write_json(
            Path(self._tmp.name), [{"path": "old.md", "commit": "0"}]
        )
        ddt._save_records([{"path": "new.md", "commit": "1"}])
        result = _read_json(Path(self._tmp.name))
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0]["path"], "new.md")


# ---------------------------------------------------------------------------
# Tests – cmd_finished
# ---------------------------------------------------------------------------


class TestCmdFinished(unittest.TestCase):
    """Tests for cmd_finished."""

    def setUp(self):
        self._orig_repo = ddt.REPO_ROOT
        self._orig_records = ddt.RECORDS_FILE
        self._tmpdir = tempfile.mkdtemp(prefix="ddt_finished_")
        self._fake_repo = Path(self._tmpdir) / "repo"
        self._fake_repo.mkdir()
        ddt.REPO_ROOT = self._fake_repo
        ddt.RECORDS_FILE = self._fake_repo / "records.json"

    def tearDown(self):
        ddt.REPO_ROOT = self._orig_repo
        ddt.RECORDS_FILE = self._orig_records
        shutil.rmtree(self._tmpdir)

    def _patch_branch(self, branch: str):
        """Return a patcher that makes _current_branch return *branch*."""
        return mock.patch.object(ddt, "_current_branch", return_value=branch)

    def _patch_head(self, commit: str = "aaaa1111"):
        return mock.patch.object(ddt, "_head_commit", return_value=commit)

    def _patch_commit_date(self, dt: str = "2025-01-01T00:00:00+00:00"):
        return mock.patch.object(ddt, "_commit_committer_date", return_value=dt)

    def _patch_now(self, iso: str = "2025-06-12T00:00:00+08:00"):
        return mock.patch.object(ddt, "_now_iso", return_value=iso)

    def test_not_on_master(self):
        """Should error (rc=1) when not on master branch."""
        args = _make_args(dir="docs/design")
        with self._patch_branch("feature/foo"):
            rc = ddt.cmd_finished(args)
        self.assertEqual(rc, 1)

    def test_directory_not_exist(self):
        """Should error when target directory doesn't exist."""
        args = _make_args(dir="nonexistent")
        with self._patch_branch("master"):
            rc = ddt.cmd_finished(args)
        self.assertEqual(rc, 1)

    def test_directory_is_file(self):
        """Should error when target is a file, not a directory."""
        target = self._fake_repo / "not_a_dir.md"
        target.write_text("hi")
        args = _make_args(dir="not_a_dir.md")
        with self._patch_branch("master"):
            rc = ddt.cmd_finished(args)
        self.assertEqual(rc, 1)

    def test_empty_directory_no_md(self):
        """Directory exists but has no .md files → rc=0, prints hint."""
        empty_dir = self._fake_repo / "empty_dir"
        empty_dir.mkdir()
        args = _make_args(dir="empty_dir")

        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            with self._patch_branch("master"):
                rc = ddt.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        self.assertIn("no .md files found", buf.getvalue())

    def test_normal_record_files(self):
        """Happy path: master branch, dir has .md files, records created."""
        design_dir = self._fake_repo / "docs" / "design"
        design_dir.mkdir(parents=True)
        (design_dir / "auth.md").write_text("# Auth")
        (design_dir / "db.md").write_text("# DB")

        args = _make_args(dir="docs/design")

        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            with self._patch_branch("master"), \
                 self._patch_head("deadbeef"), \
                 self._patch_commit_date("2025-01-01T12:00:00+00:00"), \
                 self._patch_now("2025-06-12T12:00:00+08:00"):
                rc = ddt.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        output = buf.getvalue()
        self.assertIn("2 file(s)", output)

        # Verify records.json
        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(len(records), 2)
        paths = {r["path"] for r in records}
        self.assertIn("docs/design/auth.md", paths)
        self.assertIn("docs/design/db.md", paths)
        # All records share the same commit
        for r in records:
            self.assertEqual(r["commit"], "deadbeef")
            self.assertEqual(r["comment"], "")

    def test_idempotent_record_update(self):
        """Running finished twice updates (not duplicates) records."""
        design_dir = self._fake_repo / "docs" / "design"
        design_dir.mkdir(parents=True)
        (design_dir / "auth.md").write_text("# Auth")

        args = _make_args(dir="docs/design")
        import io, sys

        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            with self._patch_branch("master"), \
                 self._patch_head("v1"), \
                 self._patch_commit_date("2025-01-01T00:00:00+00:00"), \
                 self._patch_now("2025-01-01T00:00:00+08:00"):
                ddt.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        # Second run with different commit
        sys.stdout = io.StringIO()
        try:
            with self._patch_branch("master"), \
                 self._patch_head("v2"), \
                 self._patch_commit_date("2025-02-01T00:00:00+00:00"), \
                 self._patch_now("2025-02-01T00:00:00+08:00"):
                ddt.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        records = _read_json(ddt.RECORDS_FILE)
        # Should still be exactly 1 record (updated, not duplicated)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["commit"], "v2")
        self.assertEqual(records[0]["comment"], "")


    def test_finished_clears_comment(self):
        """Re-running finished on a file with a comment clears it to ''."""
        design_dir = self._fake_repo / "docs" / "design"
        design_dir.mkdir(parents=True)
        (design_dir / "auth.md").write_text("# Auth")

        # First run – set up record, then add a comment
        args_finished = _make_args(dir="docs/design")
        args_comment = _make_args(path="docs/design/auth.md", text="original comment")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            with self._patch_branch("master"), \
                 self._patch_head("c1"), \
                 self._patch_commit_date("2025-01-01T00:00:00+00:00"), \
                 self._patch_now("2025-01-01T00:00:00+08:00"):
                ddt.cmd_finished(args_finished)
            ddt.cmd_comment(args_comment)
        finally:
            sys.stdout = old_stdout

        # Verify comment was set
        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(records[0]["comment"], "original comment")

        # Second run – finished clears comment
        args2 = _make_args(dir="docs/design")
        sys.stdout = io.StringIO()
        try:
            with self._patch_branch("master"), \
                 self._patch_head("c2"), \
                 self._patch_commit_date("2025-02-01T00:00:00+00:00"), \
                 self._patch_now("2025-02-01T00:00:00+08:00"):
                ddt.cmd_finished(args2)
        finally:
            sys.stdout = old_stdout

        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["comment"], "")
        self.assertEqual(records[0]["commit"], "c2")


# ---------------------------------------------------------------------------
# Tests – cmd_comment
# ---------------------------------------------------------------------------


class TestCmdComment(unittest.TestCase):
    """Tests for cmd_comment."""

    def setUp(self):
        self._orig_repo = ddt.REPO_ROOT
        self._orig_records = ddt.RECORDS_FILE
        self._tmpdir = tempfile.mkdtemp(prefix="ddt_comment_")
        self._fake_repo = Path(self._tmpdir) / "repo"
        self._fake_repo.mkdir()
        ddt.REPO_ROOT = self._fake_repo
        ddt.RECORDS_FILE = self._fake_repo / "records.json"

    def tearDown(self):
        ddt.REPO_ROOT = self._orig_repo
        ddt.RECORDS_FILE = self._orig_records
        shutil.rmtree(self._tmpdir)

    def test_comment_success(self):
        """Existing record → comment command updates it successfully."""
        _write_json(
            ddt.RECORDS_FILE,
            [{"path": "docs/design/auth.md", "commit": "aaa111", "comment": ""}],
        )
        args = _make_args(path="docs/design/auth.md", text="needs review")
        rc = ddt.cmd_comment(args)
        self.assertEqual(rc, 0)
        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(records[0]["comment"], "needs review")

    def test_comment_no_record(self):
        """No existing record → error (rc=1)."""
        _write_json(ddt.RECORDS_FILE, [])
        args = _make_args(path="docs/design/nonexistent.md", text="hello")
        import io, sys
        old_stderr = sys.stderr
        sys.stderr = buf = io.StringIO()
        try:
            rc = ddt.cmd_comment(args)
        finally:
            sys.stderr = old_stderr
        self.assertEqual(rc, 1)
        self.assertIn("no record", buf.getvalue())

    def test_comment_overwrites_existing(self):
        """Existing comment → overwritten with new value."""
        _write_json(
            ddt.RECORDS_FILE,
            [{"path": "docs/design/auth.md", "commit": "aaa111", "comment": "old"}],
        )
        args = _make_args(path="docs/design/auth.md", text="new")
        rc = ddt.cmd_comment(args)
        self.assertEqual(rc, 0)
        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(records[0]["comment"], "new")


# ---------------------------------------------------------------------------
# Tests – cmd_check
# ---------------------------------------------------------------------------


class TestCmdCheck(unittest.TestCase):
    """Tests for cmd_check."""

    def setUp(self):
        self._orig_repo = ddt.REPO_ROOT
        self._orig_records = ddt.RECORDS_FILE
        self._orig_design = ddt.DESIGN_DOC_DIR
        self._tmpdir = tempfile.mkdtemp(prefix="ddt_check_")
        self._fake_repo = Path(self._tmpdir) / "repo"
        self._fake_repo.mkdir()
        ddt.REPO_ROOT = self._fake_repo
        ddt.RECORDS_FILE = self._fake_repo / "records.json"
        ddt.DESIGN_DOC_DIR = self._fake_repo / "docs" / "design"

    def tearDown(self):
        ddt.REPO_ROOT = self._orig_repo
        ddt.RECORDS_FILE = self._orig_records
        ddt.DESIGN_DOC_DIR = self._orig_design
        shutil.rmtree(self._tmpdir)

    def _create_design_docs(self, names: list[str]):
        """Create .md files in the design doc dir."""
        ddt.DESIGN_DOC_DIR.mkdir(parents=True, exist_ok=True)
        for name in names:
            (ddt.DESIGN_DOC_DIR / name).write_text(f"# {name}")

    def _write_records(self, records: list[dict]):
        _write_json(ddt.RECORDS_FILE, records)

    def test_no_design_dir(self):
        """If DESIGN_DOC_DIR doesn't exist, rc=0, nothing printed."""
        args = _make_args()
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            rc = ddt.cmd_check(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        self.assertEqual(buf.getvalue().strip(), "")

    def test_no_records_all_changed(self):
        """Design docs exist but no records → all reported as changed."""
        self._create_design_docs(["auth.md", "db.md"])
        self._write_records([])
        args = _make_args()

        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            rc = ddt.cmd_check(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        lines = [l.strip() for l in buf.getvalue().strip().splitlines()]
        self.assertEqual(len(lines), 2)
        self.assertIn("docs/design/auth.md", lines)
        self.assertIn("docs/design/db.md", lines)

    def test_records_no_changes(self):
        """Records match HEAD → nothing reported as changed."""
        self._create_design_docs(["auth.md"])
        self._write_records(
            [{"path": "docs/design/auth.md", "commit": "aaa111"}]
        )
        args = _make_args()

        # Mock _run so git diff --quiet returns 0 (no change)
        with mock.patch.object(ddt, "_run") as mock_run:
            mock_run.return_value = subprocess.CompletedProcess(
                args=[], returncode=0, stdout="", stderr=""
            )
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        self.assertEqual(buf.getvalue().strip(), "")

    def test_records_with_changes(self):
        """Records exist but git diff says changed → file reported."""
        self._create_design_docs(["auth.md", "db.md"])
        self._write_records(
            [
                {"path": "docs/design/auth.md", "commit": "aaa111"},
                {"path": "docs/design/db.md", "commit": "bbb222"},
            ]
        )
        args = _make_args()

        # auth.md has no change (returncode=0), db.md changed (returncode=1)
        def fake_run(cmd, **kwargs):
            if "auth.md" in str(cmd):
                return subprocess.CompletedProcess(
                    args=cmd, returncode=0, stdout="", stderr=""
                )
            else:
                return subprocess.CompletedProcess(
                    args=cmd, returncode=1, stdout="", stderr=""
                )

        with mock.patch.object(ddt, "_run", side_effect=fake_run):
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        lines = [l.strip() for l in buf.getvalue().strip().splitlines()]
        self.assertEqual(lines, ["docs/design/db.md"])

    def test_mixed_no_record_and_changed(self):
        """Mix of unrecorded + recorded-changed files."""
        self._create_design_docs(["new.md", "old.md"])
        # Only old.md has a record
        self._write_records(
            [{"path": "docs/design/old.md", "commit": "ccc333"}]
        )
        args = _make_args()

        def fake_run(cmd, **kwargs):
            if "old.md" in str(cmd):
                return subprocess.CompletedProcess(
                    args=cmd, returncode=1, stdout="", stderr=""
                )
            return subprocess.CompletedProcess(
                args=cmd, returncode=0, stdout="", stderr=""
            )

        with mock.patch.object(ddt, "_run", side_effect=fake_run):
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        lines = [l.strip() for l in buf.getvalue().strip().splitlines()]
        self.assertEqual(len(lines), 2)
        self.assertIn("docs/design/new.md", lines)  # no record → changed
        self.assertIn("docs/design/old.md", lines)  # recorded but changed

    def test_check_output_with_comment(self):
        """Changed file with comment → output is 'path\tcomment'."""
        self._create_design_docs(["auth.md"])
        self._write_records(
            [{"path": "docs/design/auth.md", "commit": "aaa111", "comment": "important doc"}]
        )
        args = _make_args()

        with mock.patch.object(ddt, "_run") as mock_run:
            mock_run.return_value = subprocess.CompletedProcess(
                args=[], returncode=1, stdout="", stderr=""
            )
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        lines = [l for l in buf.getvalue().strip().splitlines()]
        self.assertEqual(len(lines), 1)
        self.assertEqual(lines[0], "docs/design/auth.md\timportant doc")

    def test_check_output_no_comment(self):
        """Changed file without comment → output is just path."""
        self._create_design_docs(["auth.md"])
        self._write_records(
            [{"path": "docs/design/auth.md", "commit": "aaa111", "comment": ""}]
        )
        args = _make_args()

        with mock.patch.object(ddt, "_run") as mock_run:
            mock_run.return_value = subprocess.CompletedProcess(
                args=[], returncode=1, stdout="", stderr=""
            )
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        lines = [l for l in buf.getvalue().strip().splitlines()]
        self.assertEqual(len(lines), 1)
        self.assertEqual(lines[0], "docs/design/auth.md")


# ---------------------------------------------------------------------------
# Tests – _collect_md_files
# ---------------------------------------------------------------------------


class TestCollectMdFiles(unittest.TestCase):
    """Tests for _collect_md_files."""

    def setUp(self):
        self._orig_repo = ddt.REPO_ROOT
        self._tmpdir = tempfile.mkdtemp(prefix="ddt_collect_")
        self._fake_repo = Path(self._tmpdir) / "repo"
        self._fake_repo.mkdir()
        ddt.REPO_ROOT = self._fake_repo

    def tearDown(self):
        ddt.REPO_ROOT = self._orig_repo
        shutil.rmtree(self._tmpdir)

    def test_collects_md_files_recursive(self):
        d = self._fake_repo / "a" / "b"
        d.mkdir(parents=True)
        (d / "x.md").write_text("x")
        (self._fake_repo / "a" / "y.txt").write_text("txt")
        (self._fake_repo / "a" / "z.md").write_text("z")

        result = ddt._collect_md_files(self._fake_repo / "a")
        result_str = [str(p) for p in result]
        self.assertIn("a/b/x.md", result_str)
        self.assertIn("a/z.md", result_str)
        self.assertEqual(len(result), 2)

    def test_sorted_output(self):
        d = self._fake_repo / "docs"
        d.mkdir()
        for name in ["c.md", "a.md", "b.md"]:
            (d / name).write_text(name)

        result = ddt._collect_md_files(d)
        paths = [str(p) for p in result]
        self.assertEqual(paths, sorted(paths))

    def test_no_md_files(self):
        d = self._fake_repo / "docs"
        d.mkdir()
        (d / "readme.txt").write_text("nope")
        result = ddt._collect_md_files(d)
        self.assertEqual(result, [])


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


if __name__ == "__main__":
    unittest.main()
