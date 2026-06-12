#!/usr/bin/env python3
"""
Design Doc Tracker (ddt) – track which design docs have been implemented.

Commands
--------
- ``finished <dir>``
    Record that every ``.md`` file under *<dir>* matches current HEAD.
    Must be on the ``master`` branch.  *<dir>* must exist and contain at
    least one ``.md`` file.

- ``check``
    Scan ``docs/design/`` for ``.md`` files and report any that have
    changed since their last confirmation.

records.json lives alongside this script.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List

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


def _current_branch() -> str:
    r = _run(["git", "rev-parse", "--abbrev-ref", "HEAD"])
    return r.stdout.strip()


def _head_commit() -> str:
    r = _run(["git", "rev-parse", "HEAD"])
    return r.stdout.strip()


def _commit_committer_date(ref: str = "HEAD") -> str:
    """Return ISO-8601 committer date for *ref*."""
    r = _run(["git", "log", "-1", "--format=%cI", ref])
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


def _collect_md_files(directory: Path) -> List[Path]:
    """Recursively collect .md files, return relative-to-REPO_ROOT paths."""
    base = str(REPO_ROOT) + "/"
    result: List[Path] = []
    for p in sorted(directory.rglob("*.md")):
        if p.is_file():
            rel = str(p.relative_to(REPO_ROOT))
            result.append(Path(rel))
    return result


def _now_iso() -> str:
    return datetime.now(timezone.utc).astimezone().isoformat()


# ── sub-commands ─────────────────────────────────────────────────────────


def cmd_finished(args: argparse.Namespace) -> int:
    dir_path = REPO_ROOT / args.dir

    # 1. branch must be master
    branch = _current_branch()
    if branch != "master":
        print(f"Error: must be on master branch, currently on '{branch}'", file=sys.stderr)
        return 1

    # 2. directory must exist and be a directory
    if not dir_path.exists():
        print(f"Error: directory '{args.dir}' does not exist", file=sys.stderr)
        return 1
    if not dir_path.is_dir():
        print(f"Error: '{args.dir}' is not a directory", file=sys.stderr)
        return 1

    # 3. collect .md files
    md_files = _collect_md_files(dir_path)
    if not md_files:
        print("no .md files found")
        return 0

    # 4. build records
    commit = _head_commit()
    commit_time = _commit_committer_date()
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
        }
        if key in existing:
            records[existing[key]] = entry
        else:
            records.append(entry)

    _save_records(records)
    print(f"Recorded {len(md_files)} file(s) under '{args.dir}'")
    return 0


def cmd_check(args: argparse.Namespace) -> int:
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
        # git diff --quiet exits 1 if there are changes
        r = _run(["git", "diff", "--quiet", f"{rec['commit']}..HEAD", "--", key])
        if r.returncode != 0:
            changed.append(key)

    for p in changed:
        print(p)

    return 0


# ── main ─────────────────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Design Doc Tracker – track implementation status of design docs."
    )
    sub = parser.add_subparsers(dest="command")

    p_finished = sub.add_parser("finished", help="Mark design doc(s) as implemented")
    p_finished.add_argument("dir", help="Directory (relative to repo root) to record")

    sub.add_parser("check", help="Check for changed design docs since last confirmation")

    args = parser.parse_args()
    if args.command is None:
        parser.print_help()
        return 1

    if args.command == "finished":
        return cmd_finished(args)
    elif args.command == "check":
        return cmd_check(args)
    else:
        parser.print_help()
        return 1


if __name__ == "__main__":
    sys.exit(main())
