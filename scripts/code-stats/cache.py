#!/usr/bin/env python3
"""
Cache I/O for ``collect_code_stats.py``.

Pure functions only — no git calls, no I/O outside the cache file. ``load_cache``
returns ``None`` on any structural failure (missing file, bad JSON, schema
mismatch, missing keys) so the caller can transparently fall back to a full
collect. ``save_cache`` writes atomically via ``tmp + os.replace`` to avoid
truncated files on crash.

Schema version 1
-----------------
::

    {
        "schema_version": 1,
        "head": "<short hash>",
        "generated_at": "<ISO8601>",
        "columns": {
            "dates": [...], "code_total_loc": [...], "code_changed_cum": [...],
            "doc_total_loc": [...], "code_files": [...], "doc_files": [...],
            "test_cases": [...], "test_loc": [...]
        },
        "days": {
            "YYYY-MM-DD": {"commit": "<sha>", "changed": <int>, "snap": <dict|None>},
            ...
        }
    }

Days without commits store ``{"commit": "", "changed": 0, "snap": None}``.
"""

from __future__ import annotations

import json
import os
import re

SCHEMA_VERSION = 1

_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")

_REQUIRED_COLUMNS = (
    "dates", "code_total_loc", "code_changed_cum", "doc_total_loc",
    "code_files", "doc_files", "test_cases", "test_loc",
)


def load_cache(path: str) -> dict | None:
    """Read and parse the cache file. Return ``None`` on any failure.

    A ``None`` return signals "fall back to full collect". Caller does not
    need to distinguish failure modes — all paths converge on rebuild.
    """
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, json.JSONDecodeError, ValueError):
        return None

    if not isinstance(data, dict):
        return None
    if data.get("schema_version") != SCHEMA_VERSION:
        return None

    cols = data.get("columns")
    days = data.get("days")
    if not isinstance(cols, dict) or not isinstance(days, dict):
        return None

    for key in _REQUIRED_COLUMNS:
        if key not in cols or not isinstance(cols[key], list):
            return None
        if len(cols[key]) != len(cols["dates"]):
            return None

    if "dates" not in cols or not cols["dates"]:
        return None

    return _validate_semantics(data)


def _validate_semantics(data: dict) -> dict | None:
    """Check semantic invariants; return ``data`` or ``None`` on violation.

    Guards against structurally valid JSON that would crash or silently
    corrupt downstream collection: malformed dates, unsorted dates, snap
    of wrong type, days/dates key mismatch, wrong-typed day fields.
    """
    cols = data["columns"]
    days = data["days"]

    prev = None
    for d in cols["dates"]:
        if not isinstance(d, str) or not _DATE_RE.match(d):
            return None
        if prev is not None and d <= prev:
            return None  # must be strictly ascending
        prev = d

    if set(days.keys()) != set(cols["dates"]):
        return None

    for day, rec in days.items():
        if not isinstance(rec, dict):
            return None
        if not isinstance(rec.get("commit", ""), str):
            return None
        if not isinstance(rec.get("changed", 0), int):
            return None
        snap = rec.get("snap")
        if snap is not None and not isinstance(snap, dict):
            return None

    return data


def save_cache(path: str, data: dict) -> None:
    """Atomically write ``data`` to ``path``.

    Writes to a pid-suffixed tmp file first (two concurrent writers must
    not clobber each other's tmp), then ``os.replace`` (atomic on POSIX and
    Windows, same filesystem). The parent directory is fsync'd afterwards
    so a power loss cannot orphan the renamed entry. Existing file is
    overwritten; missing parent dirs are created.
    """
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)

    tmp = f"{path}.{os.getpid()}.tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)

    if parent:
        try:
            dir_fd = os.open(parent, os.O_DIRECTORY)
            try:
                os.fsync(dir_fd)
            finally:
                os.close(dir_fd)
        except OSError:
            pass  # dir fsync is best-effort (not all FS support it)


__all__ = ["SCHEMA_VERSION", "load_cache", "save_cache"]
