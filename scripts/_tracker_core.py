#!/usr/bin/env python3
"""Shared helpers for the doc-tracker CLIs (ddt / ret).

This module holds the generic, tool-agnostic plumbing shared by the
design-doc-tracker (ddt) and requirement-e2e-tracker (ret) scripts:

- running git subprocesses inside the repo root
- computing ``git merge-base HEAD origin/master``
- reading/writing ``records.json``
- collecting ``.md`` files (with a deterministic sort order)
- detecting whether tracked paths changed since a recorded commit

It deliberately does NOT compute ``REPO_ROOT`` at import time so that both
CLI scripts (and their tests) control their own repository root and records
file.
"""

from __future__ import annotations

import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Sequence


def run(cmd: List[str], cwd: Path, **kwargs: Any) -> subprocess.CompletedProcess[str]:
    """Run *cmd* inside *cwd*, returning a CompletedProcess with text output."""
    return subprocess.run(
        cmd, cwd=cwd, capture_output=True, text=True, **kwargs
    )


def commit_committer_date(run_fn, ref: str = "HEAD") -> str:
    """Return the ISO-8601 committer date for *ref*."""
    r = run_fn(["git", "log", "-1", "--format=%cI", ref])
    return r.stdout.strip()


def merge_base_commit(run_fn) -> str | None:
    """Return the merge-base of HEAD and origin/master, or None on failure."""
    r = run_fn(["git", "merge-base", "HEAD", "origin/master"])
    if r.returncode != 0 or not r.stdout.strip():
        return None
    return r.stdout.strip()


def diff_quiet_changed(run_fn, commit: str, paths: Sequence[str]) -> bool:
    """Return True when any of *paths* differs between *commit* and HEAD.

    Mirrors ``git diff --quiet <commit>..HEAD -- <paths...>``: exit code 0
    means no change, any non-zero exit code means at least one path changed.
    """
    r = run_fn(["git", "diff", "--quiet", f"{commit}..HEAD", "--", *paths])
    return r.returncode != 0


def now_iso() -> str:
    """Return the current local time as an ISO-8601 string."""
    return datetime.now(timezone.utc).astimezone().isoformat()


def load_records(
    records_file: Path,
    default_fields: Dict[str, str] | None = None,
) -> List[Dict[str, str]]:
    """Load records.json, applying *default_fields* to any missing keys.

    Returns ``[]`` when the file does not exist.
    """
    if records_file.exists():
        with open(records_file, "r", encoding="utf-8") as f:
            records = json.load(f)
        for rec in records:
            for key, value in (default_fields or {}).items():
                rec.setdefault(key, value)
        return records
    return []


def save_records(
    records_file: Path,
    records: List[Dict[str, str]],
    key_field: str = "path",
) -> None:
    """Sort records by *key_field* and write them back to records.json."""
    records.sort(key=lambda r: r[key_field])
    with open(records_file, "w", encoding="utf-8") as f:
        json.dump(records, f, indent=2, ensure_ascii=False)
        f.write("\n")


def upsert_record(
    records: List[Dict[str, str]],
    key_field: str,
    key_value: str,
    defaults: Dict[str, str],
    **fields: str,
) -> List[Dict[str, str]]:
    """Find or create the record where *key_field* == *key_value*, then update it.

    New records start with *key_field* set, then *defaults*, then *fields*.
    Returns the mutated *records* list (the same object).
    """
    existing: Dict[str, int] = {r[key_field]: i for i, r in enumerate(records)}
    if key_value in existing:
        records[existing[key_value]].update(fields)
    else:
        entry: Dict[str, str] = {key_field: key_value}
        entry.update(defaults)
        entry.update(fields)
        records.append(entry)
    return records


def sort_key(repo_root: Path, p: Path) -> list:
    """Sort key: subdirectories before index files at each level.

    Splits the path into segments and lowercases each one for
    case-insensitive comparison.  Directory segments naturally sort before
    file-name segments at the same depth.
    """
    return [part.lower() for part in p.relative_to(repo_root).parts]


def collect_md_files(
    repo_root: Path,
    directory: Path,
    blacklist: frozenset = frozenset(),
) -> List[Path]:
    """Recursively collect ``.md`` files under *directory*.

    Returns paths relative to *repo_root*, sorted with subdirectory contents
    before index files at each level, excluding any repo-root-relative string
    present in *blacklist*.
    """
    result: List[Path] = []
    for p in sorted(directory.rglob("*.md"), key=lambda q: sort_key(repo_root, q)):
        if p.is_file():
            rel = str(p.relative_to(repo_root))
            if rel in blacklist:
                continue
            result.append(Path(rel))
    return result
