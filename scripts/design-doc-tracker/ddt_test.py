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
        # Migration adds blocked_reason to records that lack it
        expected = [{"path": "a.md", "commit": "abc", "blocked_reason": ""}]
        self.assertEqual(result, expected)

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
        # Migration adds blocked_reason to records that lack it
        expected = [
            {"path": "a.md", "commit": "a1", "blocked_reason": ""},
            {"path": "c.md", "commit": "c1", "blocked_reason": ""},
        ]
        self.assertEqual(loaded, expected)  # already sorted
        ddt._save_records(loaded)
        loaded2 = ddt._load_records()
        self.assertEqual(loaded2, expected)

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

    def _patch_merge_base(self, commit: str | None = "aaaa1111"):
        """Return a patcher that makes _merge_base_commit return *commit* (or None)."""
        return mock.patch.object(ddt, "_merge_base_commit", return_value=commit)

    def _patch_commit_date(self, dt: str = "2025-01-01T00:00:00+00:00"):
        return mock.patch.object(ddt, "_commit_committer_date", return_value=dt)

    def _patch_now(self, iso: str = "2025-06-12T00:00:00+08:00"):
        return mock.patch.object(ddt, "_now_iso", return_value=iso)

    def test_directory_not_exist(self):
        """Should error when target directory doesn't exist."""
        args = _make_args(dir="nonexistent")
        rc = ddt.cmd_finished(args)
        self.assertEqual(rc, 1)

    def test_non_md_file_rejected(self):
        """Should error when target is a non-.md file."""
        target = self._fake_repo / "not_a_dir.txt"
        target.write_text("hi")
        args = _make_args(dir="not_a_dir.txt")
        rc = ddt.cmd_finished(args)
        self.assertEqual(rc, 1)

    # -- merge-base tests ---------------------------------------------------

    def test_merge_base_success(self):
        """Happy path: merge-base returns a commit, records it."""
        design_dir = self._fake_repo / "docs" / "design"
        design_dir.mkdir(parents=True)
        (design_dir / "auth.md").write_text("# Auth")

        args = _make_args(dir="docs/design")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            with self._patch_merge_base("merge001"), \
                 self._patch_commit_date("2025-03-01T00:00:00+00:00"), \
                 self._patch_now("2025-06-12T00:00:00+08:00"):
                rc = ddt.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        self.assertIn("1 file(s)", buf.getvalue())

        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["commit"], "merge001")
        self.assertEqual(records[0]["comment"], "")

    def test_merge_base_failure(self):
        """Should error (rc=1) when merge-base fails."""
        # Create directory so it passes the existence check
        design_dir = self._fake_repo / "docs" / "design"
        design_dir.mkdir(parents=True)
        (design_dir / "auth.md").write_text("# Auth")
        args = _make_args(dir="docs/design")
        import io, sys
        old_stderr = sys.stderr
        sys.stderr = buf = io.StringIO()
        try:
            with self._patch_merge_base(None):
                rc = ddt.cmd_finished(args)
        finally:
            sys.stderr = old_stderr
        self.assertEqual(rc, 1)
        self.assertIn("merge-base", buf.getvalue())

    # -- single file tests ---------------------------------------------------

    def test_single_md_file_happy_path(self):
        """Passing a .md file path should record only that file."""
        design_dir = self._fake_repo / "docs" / "design"
        design_dir.mkdir(parents=True)
        (design_dir / "auth.md").write_text("# Auth")
        (design_dir / "db.md").write_text("# DB")

        args = _make_args(dir="docs/design/auth.md")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            with self._patch_merge_base("cafe0001"), \
                 self._patch_commit_date("2025-03-01T00:00:00+00:00"), \
                 self._patch_now("2025-06-12T00:00:00+08:00"):
                rc = ddt.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        output = buf.getvalue()
        self.assertIn("1 file(s)", output)

        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["path"], "docs/design/auth.md")
        self.assertEqual(records[0]["commit"], "cafe0001")

    def test_single_non_md_file_error(self):
        """Passing a non-.md file should error."""
        target = self._fake_repo / "readme.txt"
        target.write_text("hello")
        args = _make_args(dir="readme.txt")
        import io, sys
        old_stderr = sys.stderr
        sys.stderr = buf = io.StringIO()
        try:
            rc = ddt.cmd_finished(args)
        finally:
            sys.stderr = old_stderr
        self.assertEqual(rc, 1)
        self.assertIn("not a .md file", buf.getvalue())

    def test_single_md_file_not_exist_error(self):
        """Passing a non-existent .md path should error."""
        args = _make_args(dir="docs/design/missing.md")
        import io, sys
        old_stderr = sys.stderr
        sys.stderr = buf = io.StringIO()
        try:
            rc = ddt.cmd_finished(args)
        finally:
            sys.stderr = old_stderr
        self.assertEqual(rc, 1)
        self.assertIn("does not exist", buf.getvalue())

    def test_directory_behavior_unchanged(self):
        """Directory path should still work as before (recursive)."""
        design_dir = self._fake_repo / "docs" / "design"
        design_dir.mkdir(parents=True)
        (design_dir / "auth.md").write_text("# Auth")
        (design_dir / "db.md").write_text("# DB")

        args = _make_args(dir="docs/design")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            with self._patch_merge_base("dir0001"), \
                 self._patch_commit_date("2025-04-01T00:00:00+00:00"), \
                 self._patch_now("2025-06-12T00:00:00+08:00"):
                rc = ddt.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        self.assertIn("2 file(s)", buf.getvalue())

        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(len(records), 2)
        paths = {r["path"] for r in records}
        self.assertIn("docs/design/auth.md", paths)
        self.assertIn("docs/design/db.md", paths)

    def test_empty_directory_no_md(self):
        """Directory exists but has no .md files → rc=0, prints hint."""
        empty_dir = self._fake_repo / "empty_dir"
        empty_dir.mkdir()
        args = _make_args(dir="empty_dir")

        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            rc = ddt.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        self.assertIn("no .md files found", buf.getvalue())

    def test_normal_record_files(self):
        """Happy path: dir has .md files, records created from merge-base."""
        design_dir = self._fake_repo / "docs" / "design"
        design_dir.mkdir(parents=True)
        (design_dir / "auth.md").write_text("# Auth")
        (design_dir / "db.md").write_text("# DB")

        args = _make_args(dir="docs/design")

        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            with self._patch_merge_base("deadbeef"), \
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
            with self._patch_merge_base("v1"), \
                 self._patch_commit_date("2025-01-01T00:00:00+00:00"), \
                 self._patch_now("2025-01-01T00:00:00+08:00"):
                ddt.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        # Second run with different commit
        sys.stdout = io.StringIO()
        try:
            with self._patch_merge_base("v2"), \
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
            with self._patch_merge_base("c1"), \
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
            with self._patch_merge_base("c2"), \
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

    def test_comment_creates_record_for_unrecorded_file(self):
        """No existing record → creates a new record with empty commit."""
        _write_json(ddt.RECORDS_FILE, [])
        args = _make_args(path="docs/design/new.md", text="first comment")
        rc = ddt.cmd_comment(args)
        self.assertEqual(rc, 0)
        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["path"], "docs/design/new.md")
        self.assertEqual(records[0]["commit"], "")
        self.assertEqual(records[0]["commit_time"], "")
        self.assertEqual(records[0]["comment"], "first comment")
        self.assertNotEqual(records[0]["confirmed_time"], "")

    def test_comment_empty_string(self):
        """Empty string comment should be written successfully."""
        _write_json(
            ddt.RECORDS_FILE,
            [{"path": "docs/design/auth.md", "commit": "aaa111", "comment": "old"}],
        )
        args = _make_args(path="docs/design/auth.md", text="")
        rc = ddt.cmd_comment(args)
        self.assertEqual(rc, 0)
        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(records[0]["comment"], "")

    def test_comment_unrecorded_keeps_empty_commit(self):
        """Creating a record via comment should have empty commit."""
        _write_json(ddt.RECORDS_FILE, [])
        args = _make_args(path="docs/design/brand_new.md", text="needs review")
        rc = ddt.cmd_comment(args)
        self.assertEqual(rc, 0)
        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(records[0]["commit"], "")
        self.assertEqual(records[0]["comment"], "needs review")

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

    def test_empty_commit_treated_as_changed(self):
        """Record with empty commit → treated as changed (no git diff run)."""
        self._create_design_docs(["auth.md"])
        self._write_records(
            [{"path": "docs/design/auth.md", "commit": "", "comment": ""}]
        )
        args = _make_args()

        # _run should NOT be called since empty commit skips git diff
        with mock.patch.object(ddt, "_run") as mock_run:
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        mock_run.assert_not_called()
        self.assertIn("docs/design/auth.md", buf.getvalue())

    def test_empty_commit_with_comment_output(self):
        """Record with empty commit + comment → output 'path\tcomment'."""
        self._create_design_docs(["auth.md"])
        self._write_records(
            [{"path": "docs/design/auth.md", "commit": "", "comment": "needs implementation"}]
        )
        args = _make_args()

        with mock.patch.object(ddt, "_run") as mock_run:
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        mock_run.assert_not_called()
        lines = [l for l in buf.getvalue().strip().splitlines()]
        self.assertEqual(len(lines), 1)
        self.assertEqual(lines[0], "docs/design/auth.md\tneeds implementation")

    def test_empty_commit_mixed_with_normal(self):
        """Mix of empty-commit and normal-commit records."""
        self._create_design_docs(["new.md", "ok.md", "old.md"])
        self._write_records([
            {"path": "docs/design/new.md", "commit": "", "comment": ""},
            {"path": "docs/design/ok.md", "commit": "aaa111", "comment": ""},
            {"path": "docs/design/old.md", "commit": "bbb222", "comment": "stale"},
        ])
        args = _make_args()

        def fake_run(cmd, **kwargs):
            if "ok.md" in str(cmd):
                return subprocess.CompletedProcess(
                    args=cmd, returncode=0, stdout="", stderr=""
                )
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
        output = buf.getvalue()
        lines = [l.strip() for l in output.strip().splitlines()]
        # new.md (empty commit) and old.md (changed) should be reported; ok.md should not
        self.assertEqual(len(lines), 2)
        self.assertIn("docs/design/new.md", lines)
        self.assertIn("docs/design/old.md\tstale", lines)
        self.assertNotIn("docs/design/ok.md", output)


# ---------------------------------------------------------------------------
# Tests – cmd_blocked
# ---------------------------------------------------------------------------


class TestCmdBlocked(unittest.TestCase):
    """Tests for cmd_blocked."""

    def setUp(self):
        self._orig_repo = ddt.REPO_ROOT
        self._orig_records = ddt.RECORDS_FILE
        self._tmpdir = tempfile.mkdtemp(prefix="ddt_blocked_")
        self._fake_repo = Path(self._tmpdir) / "repo"
        self._fake_repo.mkdir()
        ddt.REPO_ROOT = self._fake_repo
        ddt.RECORDS_FILE = self._fake_repo / "records.json"

    def tearDown(self):
        ddt.REPO_ROOT = self._orig_repo
        ddt.RECORDS_FILE = self._orig_records
        shutil.rmtree(self._tmpdir)

    def _patch_now(self, iso: str = "2025-06-12T00:00:00+08:00"):
        return mock.patch.object(ddt, "_now_iso", return_value=iso)

    def test_blocked_creates_new_record(self):
        """No existing record → creates a new one with blocked_reason."""
        _write_json(ddt.RECORDS_FILE, [])
        args = _make_args(path="docs/design/auth.md", reason="waiting on API")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            with self._patch_now():
                rc = ddt.cmd_blocked(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        self.assertIn("Created blocked record", buf.getvalue())
        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["path"], "docs/design/auth.md")
        self.assertEqual(records[0]["blocked_reason"], "waiting on API")
        self.assertEqual(records[0]["commit"], "")
        self.assertEqual(records[0]["comment"], "")

    def test_blocked_updates_existing_record(self):
        """Existing record → blocked_reason is updated, other fields preserved."""
        _write_json(
            ddt.RECORDS_FILE,
            [{
                "path": "docs/design/auth.md",
                "commit": "aaa111",
                "commit_time": "2025-01-01T00:00:00+00:00",
                "confirmed_time": "2025-01-01T00:00:00+08:00",
                "comment": "",
                "blocked_reason": "",
            }],
        )
        args = _make_args(path="docs/design/auth.md", reason="blocked by dep")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            with self._patch_now("2025-07-01T00:00:00+08:00"):
                rc = ddt.cmd_blocked(args)
        finally:
            sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        self.assertIn("Updated blocked reason", buf.getvalue())
        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["blocked_reason"], "blocked by dep")
        self.assertEqual(records[0]["commit"], "aaa111")  # preserved
        self.assertEqual(records[0]["confirmed_time"], "2025-07-01T00:00:00+08:00")

    def test_blocked_overwrites_existing_reason(self):
        """Existing blocked_reason → overwritten with new reason."""
        _write_json(
            ddt.RECORDS_FILE,
            [{
                "path": "docs/design/auth.md",
                "commit": "aaa111",
                "commit_time": "",
                "confirmed_time": "2025-01-01T00:00:00+08:00",
                "comment": "",
                "blocked_reason": "old reason",
            }],
        )
        args = _make_args(path="docs/design/auth.md", reason="new reason")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            with self._patch_now():
                ddt.cmd_blocked(args)
        finally:
            sys.stdout = old_stdout

        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(records[0]["blocked_reason"], "new reason")

    def test_blocked_preserves_existing_comment(self):
        """Existing comment should not be cleared by blocked."""
        _write_json(
            ddt.RECORDS_FILE,
            [{
                "path": "docs/design/auth.md",
                "commit": "aaa111",
                "commit_time": "",
                "confirmed_time": "2025-01-01T00:00:00+08:00",
                "comment": "important doc",
                "blocked_reason": "",
            }],
        )
        args = _make_args(path="docs/design/auth.md", reason="waiting")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            with self._patch_now():
                ddt.cmd_blocked(args)
        finally:
            sys.stdout = old_stdout

        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(records[0]["comment"], "important doc")
        self.assertEqual(records[0]["blocked_reason"], "waiting")

    def test_blocked_idempotent(self):
        """Running blocked twice with same args → single record updated."""
        _write_json(ddt.RECORDS_FILE, [])
        args = _make_args(path="docs/design/auth.md", reason="same reason")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            with self._patch_now():
                ddt.cmd_blocked(args)
                ddt.cmd_blocked(args)
        finally:
            sys.stdout = old_stdout

        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["blocked_reason"], "same reason")

    def test_finished_clears_blocked_reason(self):
        """cmd_finished should clear blocked_reason to empty."""
        _write_json(
            ddt.RECORDS_FILE,
            [{
                "path": "docs/design/auth.md",
                "commit": "aaa111",
                "commit_time": "2025-01-01T00:00:00+00:00",
                "confirmed_time": "2025-01-01T00:00:00+08:00",
                "comment": "",
                "blocked_reason": "blocked because X",
            }],
        )

        design_dir = self._fake_repo / "docs" / "design"
        design_dir.mkdir(parents=True)
        (design_dir / "auth.md").write_text("# Auth")

        args = _make_args(dir="docs/design")
        import io, sys
        old_stdout = sys.stdout
        sys.stdout = io.StringIO()
        try:
            with mock.patch.object(ddt, "_merge_base_commit", return_value="newcommit"), \
                 mock.patch.object(ddt, "_commit_committer_date", return_value="2025-07-01T00:00:00+00:00"), \
                 mock.patch.object(ddt, "_now_iso", return_value="2025-07-01T00:00:00+08:00"):
                ddt.cmd_finished(args)
        finally:
            sys.stdout = old_stdout

        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(records[0]["blocked_reason"], "")


# ---------------------------------------------------------------------------
# Tests – cmd_check with blocked docs
# ---------------------------------------------------------------------------


class TestCmdCheckBlocked(unittest.TestCase):
    """Tests for cmd_check behavior with blocked documents."""

    def setUp(self):
        self._orig_repo = ddt.REPO_ROOT
        self._orig_records = ddt.RECORDS_FILE
        self._orig_design = ddt.DESIGN_DOC_DIR
        self._tmpdir = tempfile.mkdtemp(prefix="ddt_check_blocked_")
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
        ddt.DESIGN_DOC_DIR.mkdir(parents=True, exist_ok=True)
        for name in names:
            (ddt.DESIGN_DOC_DIR / name).write_text(f"# {name}")

    def _write_records(self, records: list[dict]):
        _write_json(ddt.RECORDS_FILE, records)

    def test_blocked_unchanged_not_reported(self):
        """Blocked doc with no git changes → should NOT appear in output."""
        self._create_design_docs(["auth.md"])
        self._write_records([
            {
                "path": "docs/design/auth.md",
                "commit": "aaa111",
                "commit_time": "2025-01-01T00:00:00+00:00",
                "confirmed_time": "2025-01-01T00:00:00+08:00",
                "comment": "",
                "blocked_reason": "waiting on dependency",
            }
        ])
        args = _make_args()

        # git diff --quiet returns 0 → no change
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

    def test_blocked_changed_auto_unblocks(self):
        """Blocked doc with git changes → auto-unblock, reported as normal change."""
        self._create_design_docs(["auth.md"])
        self._write_records([
            {
                "path": "docs/design/auth.md",
                "commit": "aaa111",
                "commit_time": "2025-01-01T00:00:00+00:00",
                "confirmed_time": "2025-01-01T00:00:00+08:00",
                "comment": "",
                "blocked_reason": "waiting on dependency",
            }
        ])
        args = _make_args()

        def fake_run(cmd, **kwargs):
            # git diff --quiet → returncode=1 means changed
            return subprocess.CompletedProcess(
                args=cmd, returncode=1, stdout="", stderr=""
            )

        with mock.patch.object(ddt, "_run", side_effect=fake_run), \
             mock.patch.object(ddt, "_merge_base_commit", return_value="newcommit"), \
             mock.patch.object(ddt, "_commit_committer_date", return_value="2025-07-01T00:00:00+00:00"), \
             mock.patch.object(ddt, "_now_iso", return_value="2025-07-01T00:00:00+08:00"):
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        output = buf.getvalue().strip()
        self.assertEqual(output, "docs/design/auth.md")

        # Verify record was updated: blocked_reason cleared, commit updated
        records = _read_json(ddt.RECORDS_FILE)
        self.assertEqual(records[0]["blocked_reason"], "")
        self.assertEqual(records[0]["commit"], "newcommit")

    def test_blocked_changed_reported_without_blocked_marker(self):
        """Auto-unblocked doc should not show blocked_reason in output."""
        self._create_design_docs(["auth.md"])
        self._write_records([
            {
                "path": "docs/design/auth.md",
                "commit": "aaa111",
                "commit_time": "",
                "confirmed_time": "",
                "comment": "some comment",
                "blocked_reason": "was blocked",
            }
        ])
        args = _make_args()

        def fake_run(cmd, **kwargs):
            return subprocess.CompletedProcess(
                args=cmd, returncode=1, stdout="", stderr=""
            )

        with mock.patch.object(ddt, "_run", side_effect=fake_run), \
             mock.patch.object(ddt, "_merge_base_commit", return_value="newcommit"), \
             mock.patch.object(ddt, "_commit_committer_date", return_value="2025-07-01T00:00:00+00:00"), \
             mock.patch.object(ddt, "_now_iso", return_value="2025-07-01T00:00:00+08:00"):
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        lines = [l.strip() for l in buf.getvalue().strip().splitlines()]
        # Should show comment (if any) but not blocked_reason
        self.assertEqual(len(lines), 1)
        self.assertEqual(lines[0], "docs/design/auth.md\tsome comment")

    def test_mixed_blocked_and_normal(self):
        """Mix of blocked-unchanged, blocked-changed, and normal-changed."""
        self._create_design_docs(["a.md", "b.md", "c.md"])
        self._write_records([
            {
                "path": "docs/design/a.md", "commit": "aaa",
                "commit_time": "", "confirmed_time": "",
                "comment": "", "blocked_reason": "blocked A",
            },
            {
                "path": "docs/design/b.md", "commit": "bbb",
                "commit_time": "", "confirmed_time": "",
                "comment": "", "blocked_reason": "blocked B",
            },
            {
                "path": "docs/design/c.md", "commit": "ccc",
                "commit_time": "", "confirmed_time": "",
                "comment": "", "blocked_reason": "",
            },
        ])
        args = _make_args()

        def fake_run(cmd, **kwargs):
            cmd_str = " ".join(cmd)
            if "a.md" in cmd_str:
                # blocked + unchanged
                return subprocess.CompletedProcess(args=cmd, returncode=0)
            # b.md and c.md → changed
            return subprocess.CompletedProcess(args=cmd, returncode=1)

        with mock.patch.object(ddt, "_run", side_effect=fake_run), \
             mock.patch.object(ddt, "_merge_base_commit", return_value="newcommit"), \
             mock.patch.object(ddt, "_commit_committer_date", return_value="2025-07-01T00:00:00+00:00"), \
             mock.patch.object(ddt, "_now_iso", return_value="2025-07-01T00:00:00+08:00"):
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        lines = [l.strip() for l in buf.getvalue().strip().splitlines()]
        # a.md blocked+unchanged → NOT reported
        self.assertNotIn("docs/design/a.md", lines)
        # b.md blocked+changed → reported (auto-unblocked)
        self.assertIn("docs/design/b.md", lines)
        # c.md normal+changed → reported
        self.assertIn("docs/design/c.md", lines)
        self.assertEqual(len(lines), 2)

        # Verify b.md was unblocked
        records = _read_json(ddt.RECORDS_FILE)
        by_path = {r["path"]: r for r in records}
        self.assertEqual(by_path["docs/design/b.md"]["blocked_reason"], "")
        self.assertEqual(by_path["docs/design/b.md"]["commit"], "newcommit")
        # a.md still blocked
        self.assertEqual(by_path["docs/design/a.md"]["blocked_reason"], "blocked A")

    def test_blocked_with_empty_commit_treated_as_changed(self):
        """Blocked doc with empty commit → treated as changed (auto-unblock)."""
        self._create_design_docs(["auth.md"])
        self._write_records([
            {
                "path": "docs/design/auth.md", "commit": "",
                "commit_time": "", "confirmed_time": "",
                "comment": "", "blocked_reason": "waiting",
            }
        ])
        args = _make_args()

        # _run should NOT be called (empty commit skips git diff)
        with mock.patch.object(ddt, "_run") as mock_run, \
             mock.patch.object(ddt, "_merge_base_commit", return_value="newcommit"), \
             mock.patch.object(ddt, "_commit_committer_date", return_value="2025-07-01T00:00:00+00:00"), \
             mock.patch.object(ddt, "_now_iso", return_value="2025-07-01T00:00:00+08:00"):
            import io, sys
            old_stdout = sys.stdout
            sys.stdout = buf = io.StringIO()
            try:
                rc = ddt.cmd_check(args)
            finally:
                sys.stdout = old_stdout

        self.assertEqual(rc, 0)
        # Empty commit → treated as changed, so it's reported
        self.assertIn("docs/design/auth.md", buf.getvalue())
        # blocked_reason must be cleared by auto-unblock
        records = ddt._load_records()
        rec = next(r for r in records if r["path"] == "docs/design/auth.md")
        self.assertEqual(rec["blocked_reason"], "")



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
