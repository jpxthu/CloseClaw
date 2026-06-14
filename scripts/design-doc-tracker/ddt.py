#!/usr/bin/env python3
"""
Design Doc Tracker (ddt) – track which design docs have been implemented.

Commands
--------
- ``finished <path>``
    Record that every ``.md`` file under *<path>* matches the merge-base
    of HEAD and origin/master.
    *<path>* can be a directory (recursively) or a single ``.md`` file.
    Clears any existing comment for matched files.

- ``comment <path> <text>``
    Override the comment for a specific design doc file.  If the file already
    has a record the comment is updated; otherwise a new record is created
    with an empty commit.  ``<path>`` is relative to the repo root.

- ``check``
    Scan ``docs/design/`` for ``.md`` files and report any that have
    changed since their last confirmation.

records.json lives alongside this script.
"""

from __future__ import annotations

import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Dict, List

import click

SCRIPT_DIR = Path(__file__).parent.resolve()
REPO_ROOT = Path(
    subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=SCRIPT_DIR,
        text=True,
    ).strip()
)
RECORDS_FILE = SCRIPT_DIR / "records.json"
DESIGN_DOC_DIR = REPO_ROOT / "docs" / "design"

# ── helpers ──────────────────────────────────────────────────────────────


def _run(cmd: List[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
    """Run a command inside the repo root, return CompletedProcess."""
    return subprocess.run(
        cmd, cwd=REPO_ROOT, capture_output=True, text=True, **kwargs
    )


def _commit_committer_date(ref: str = "HEAD") -> str:
    """Return ISO-8601 committer date for *ref*."""
    r = _run(["git", "log", "-1", "--format=%cI", ref])
    return r.stdout.strip()


def _merge_base_commit() -> str | None:
    """Return the merge-base of HEAD and origin/master, or None on failure."""
    r = _run(["git", "merge-base", "HEAD", "origin/master"])
    if r.returncode != 0 or not r.stdout.strip():
        return None
    return r.stdout.strip()


def _load_records() -> List[Dict[str, str]]:
    if RECORDS_FILE.exists():
        with open(RECORDS_FILE, "r", encoding="utf-8") as f:
            return json.load(f)
    return []


def _save_records(records: List[Dict[str, str]]) -> None:
    records.sort(key=lambda r: r["path"])
    with open(RECORDS_FILE, "w", encoding="utf-8") as f:
        json.dump(records, f, indent=2, ensure_ascii=False)
        f.write("\n")


# Paths excluded from check output (relative to REPO_ROOT)
BLACKLIST = frozenset({
    "docs/design/README.md",
})


def _sort_key(p: Path) -> list:
    """Sort key: subdirectories before index files at each level.

    Splits the path into segments and lowercases each one for
    case-insensitive comparison.  Directory segments (which precede a
    ``/`` in the original string) naturally sort before file-name
    segments at the same depth, so
    ``docs/design/agent/README.md`` sorts before
    ``docs/design/README.md``.
    """
    return [part.lower() for part in p.relative_to(REPO_ROOT).parts]


def _collect_md_files(directory: Path) -> List[Path]:
    """Recursively collect .md files, return relative-to-REPO_ROOT paths.

    Results are sorted with subdirectory contents before index files
    at each level, and BLACKLIST entries are excluded.
    """
    result: List[Path] = []
    for p in sorted(directory.rglob("*.md"), key=_sort_key):
        if p.is_file():
            rel = str(p.relative_to(REPO_ROOT))
            if rel in BLACKLIST:
                continue
            result.append(Path(rel))
    return result


def _now_iso() -> str:
    return datetime.now(timezone.utc).astimezone().isoformat()


# ── sub-commands ─────────────────────────────────────────────────────────


def cmd_finished(args: SimpleNamespace) -> int:
    target = REPO_ROOT / args.dir

    # 1. resolve target: file or directory
    md_files: List[Path] = []
    if target.is_file():
        # single file: must be .md
        if target.suffix != ".md":
            print(f"Error: '{args.dir}' is not a .md file", file=sys.stderr)
            return 1
        rel = str(target.relative_to(REPO_ROOT))
        md_files = [Path(rel)]
    elif target.is_dir():
        md_files = _collect_md_files(target)
        if not md_files:
            print("no .md files found")
            return 0
    else:
        # path doesn't exist
        if target.suffix == ".md":
            print(f"Error: file '{args.dir}' does not exist", file=sys.stderr)
        else:
            print(f"Error: directory '{args.dir}' does not exist", file=sys.stderr)
        return 1

    # 2. get commit via merge-base
    commit = _merge_base_commit()
    if commit is None:
        print(
            "Error: git merge-base HEAD origin/master failed. "
            "Ensure origin/master exists.",
            file=sys.stderr,
        )
        return 1

    # 3. build records
    commit_time = _commit_committer_date(commit)
    confirmed_time = _now_iso()

    records = _load_records()
    existing: Dict[str, int] = {r["path"]: i for i, r in enumerate(records)}

    for rel_path in md_files:
        key = str(rel_path)
        entry: Dict[str, str] = {
            "path": key,
            "commit": commit,
            "commit_time": commit_time,
            "confirmed_time": confirmed_time,
            "comment": "",
        }
        if key in existing:
            records[existing[key]] = entry
        else:
            records.append(entry)

    _save_records(records)
    print(f"Recorded {len(md_files)} file(s) under '{args.dir}'")
    return 0


def cmd_comment(args: SimpleNamespace) -> int:
    """Override the comment for a single design doc file.

    If the file already has a record, only the comment is overwritten.
    If no record exists yet, a new record is created with an empty commit.
    """
    records = _load_records()
    for rec in records:
        if rec["path"] == args.path:
            rec["comment"] = args.text
            _save_records(records)
            print(f"Updated comment for '{args.path}'")
            return 0
    # No existing record — create a new one with empty commit fields
    records.append(
        {
            "path": args.path,
            "commit": "",
            "commit_time": "",
            "confirmed_time": _now_iso(),
            "comment": args.text,
        }
    )
    _save_records(records)
    print(f"Created record for '{args.path}'")
    return 0


def cmd_check(args: SimpleNamespace) -> int:
    records = _load_records()
    record_map: Dict[str, Dict[str, str]] = {r["path"]: r for r in records}

    if not DESIGN_DOC_DIR.exists():
        # nothing to check
        return 0

    md_files = _collect_md_files(DESIGN_DOC_DIR)
    changed: List[str] = []

    for rel_path in md_files:
        key = str(rel_path)
        rec = record_map.get(key)
        if rec is None:
            # no record → treat as changed
            changed.append(key)
            continue
        if rec["commit"] == "":
            # empty commit → treat as changed
            changed.append(key)
            continue
        # git diff --quiet exits 1 if there are changes
        r = _run(["git", "diff", "--quiet", f"{rec['commit']}..HEAD", "--", key])
        if r.returncode != 0:
            changed.append(key)

    for p in changed:
        rec = record_map.get(p, {})
        comment = rec.get("comment", "")
        if comment:
            print(f"{p}\t{comment}")
        else:
            print(p)

    return 0


# ── main ─────────────────────────────────────────────────────────────────


@click.group()
def main() -> int:
    """Design Doc Tracker – 跟踪设计文档的实现状态。"""
    return 0


@main.command(name="finished")
@click.argument("path")
def finished_cmd(path: str) -> int:
    """标记设计文档已实现。PATH 为仓库根目录下的文件或目录路径（支持单个 .md 文件或整个目录）。"""
    return cmd_finished(SimpleNamespace(dir=path))


@main.command(name="comment")
@click.argument("path")
@click.argument("text")
def comment_cmd(path: str, text: str) -> int:
    """为已记录的设计文档设置/覆盖评论。PATH 为文件路径，TEXT 为评论内容。"""
    return cmd_comment(SimpleNamespace(path=path, text=text))


@main.command(name="check")
def check_cmd() -> int:
    """扫描设计文档目录，报告有变更的文件。"""
    return cmd_check(SimpleNamespace())


if __name__ == "__main__":
    sys.exit(main())
