#!/usr/bin/env python3
"""Shared helpers for the doc-tracker CLIs (ddt / ret).

This module holds the generic, tool-agnostic plumbing shared by the
design-doc-tracker (ddt) and requirement-e2e-tracker (ret) scripts:

- running git subprocesses inside the repo root
- computing ``git merge-base HEAD origin/master``
- reading/writing ``records.json``
- collecting ``.md`` files (with a deterministic sort order)
- detecting whether tracked keys changed since a recorded commit
- the ``check`` state machine (untracked / empty-commit / changed /
  auto-unblock) shared by both CLIs

It deliberately does NOT compute ``REPO_ROOT`` at import time so that both
CLI scripts (and their tests) control their own repository root and records
file.
"""

from __future__ import annotations

import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Dict, List, Sequence

# Default fields applied to every new record (migration also adds any of
# these that are missing when records.json is loaded).
RECORD_DEFAULT_FIELDS: Dict[str, str] = {
    "commit": "",
    "commit_time": "",
    "confirmed_time": "",
    "comment": "",
    "blocked_reason": "",
}

# Extra fields backfilled onto existing records at load time (older
# records.json files predate these keys).
RECORD_MIGRATION_FIELDS: Dict[str, str] = {"blocked_reason": ""}


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
    recursive: bool = True,
) -> List[Path]:
    """Collect ``.md`` files under *directory* (``rglob`` or flat ``glob``).

    Returns paths relative to *repo_root*, sorted with subdirectory
    contents before index files at each level, excluding any
    repo-root-relative string present in *blacklist*.
    """
    pattern = "**/*.md" if recursive else "*.md"
    result: List[Path] = []
    for p in sorted(directory.glob(pattern), key=lambda q: sort_key(repo_root, q)):
        if p.is_file():
            rel = str(p.relative_to(repo_root))
            if rel in blacklist:
                continue
            result.append(Path(rel))
    return result


def run_check(
    *,
    key_field: str,
    discover_keys: Callable[[], List[str]],
    key_changed: Callable[[str, str], bool],
    load_records: Callable[[], List[Dict[str, str]]],
    save_records: Callable[[List[Dict[str, str]]], None],
    upsert_record: Callable[..., List[Dict[str, str]]],
    merge_base_commit: Callable[[], str | None],
    commit_committer_date: Callable[[str], str],
    now_iso: Callable[[], str],
) -> List[str]:
    """Run the shared ``check`` state machine, returning the reported keys.

    For every key discovered by *discover_keys*:

    - no record, or a record with an empty commit → treated as changed;
      a blocked record with an empty commit is auto-unblocked first
      (blocked_reason cleared, file saved, record map refreshed)
    - *key_changed*(record's commit, key) is True → treated as changed;
      a blocked record is auto-unblocked (blocked_reason cleared, commit
      set to the fresh merge-base, file saved, record map refreshed)
    - otherwise → not changed; blocked records are silently skipped

    Returns the reported keys sorted in discovery order; the caller prints
    each as ``key`` or ``key\\tcomment``.
    """
    records = load_records()
    record_map: Dict[str, Dict[str, str]] = {r[key_field]: r for r in records}

    changed: List[str] = []

    for key in discover_keys():
        rec = record_map.get(key)
        if rec is None:
            # no record → treat as changed
            changed.append(key)
            continue
        if rec["commit"] == "":
            # empty commit → treat as changed
            blocked = rec.get("blocked_reason", "")
            if blocked:
                # blocked with empty commit → auto-unblock
                upsert_record(records, key, blocked_reason="")
                save_records(records)
                record_map = {r[key_field]: r for r in records}
            changed.append(key)
            continue
        if key_changed(rec["commit"], key):
            # key changed since last record
            blocked = rec.get("blocked_reason", "")
            if blocked:
                # blocked but updated → auto-unblock
                new_commit = merge_base_commit() or rec["commit"]
                upsert_record(
                    records, key,
                    blocked_reason="",
                    commit=new_commit,
                    commit_time=commit_committer_date(new_commit),
                    confirmed_time=now_iso(),
                )
                save_records(records)
                record_map = {r[key_field]: r for r in records}
            changed.append(key)
        else:
            # no change — skip blocked keys entirely
            blocked = rec.get("blocked_reason", "")
            if blocked:
                continue

    return changed
