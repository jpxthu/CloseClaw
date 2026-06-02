#!/usr/bin/env python3
"""
Collect daily code statistics from the CloseClaw repository.

Strategy
--------
- Walk all commits on the default branch (oldest -> newest) using
  ``git log --format='%aI %H %P'``.
- Group commits by author date (UTC+offset, normalized to YYYY-MM-DD).
- For each calendar day, the *earliest* commit of that day is the snapshot
  anchor. Days with no commit forward-fill the previous day's snapshot.
- Snapshot stats (per anchor commit):
    * code file count, code LOC (excluding blank lines, line-start ``//`` and
      ``/* */`` block comments), Rust ``#[test]`` attribute count
    * doc file count, doc total lines (no filtering)
- Cumulative change stats (per *every* commit that day, not just the anchor):
    * for each commit run ``git diff <parent> <commit> --numstat``
    * for the root commit, diff against the empty tree
    * sum ``|added| + |removed|`` for code files only, accumulated across days
      as a running total.

Output
------
Pure in-memory. ``get_data()`` returns a dict ready to be consumed by
``draw_stats.py``:

    {
        "dates":           ["2026-03-21", ...],
        "code_total_loc":  [...],   # curve 1-1 (snapshot)
        "code_changed_cum":[...],   # curve 1-2 (running total of |diff|)
        "doc_total_loc":   [...],   # curve 2
        "code_files":      [...],   # curve 3-1
        "doc_files":       [...],   # curve 3-2
        "test_cases":      [...],   # curve 4
    }

No JSONL persistence. The first call to ``get_data()`` triggers collection;
subsequent calls in the same process recompute (the function is intentionally
side-effect-free at module scope).
"""

from __future__ import annotations

import re
import subprocess
from collections import defaultdict
from datetime import datetime, timedelta
from pathlib import Path
from typing import Dict, List

SCRIPT_DIR = Path(__file__).parent.resolve()
REPO = SCRIPT_DIR.parent.parent  # closeclaw repo root

# ---------- File classification -------------------------------------------------

# Code file extensions (counted as "code" for both snapshot and diff stats).
CODE_EXTS = {".rs", ".py", ".sh", ".js"}

# Doc file extensions.
DOC_EXTS = {".md"}

# Excluded extensions (matches the plan's exclusion list).
EXCLUDED_EXTS = {".json", ".txt", ".yml", ".yaml", ".toml", ".lock", ".jsonl"}

# Excluded file basenames (case-insensitive).
EXCLUDED_FILENAMES = {".gitignore"}

# Excluded filename suffixes (case-insensitive).
EXCLUDED_SUFFIXES = (".example",)

# Excluded path prefixes (relative to repo root).
EXCLUDED_PATH_PREFIXES = ("githooks/",)

# ---------- Test attribute detection (Rust only) --------------------------------

# Matches attribute lines like:
#   #[test]                - yes
#   #[test]                - yes
#   #[test (flavor = "x")] - yes (test attribute with parenthesized args)
#   #[test                 - yes (opening on its own line, content follows)
#   #[tokio::test]         - yes
#   #[async_std::test]     - yes
#   #[test_suite]          - no  ('_' is a word char, blocks \b)
#   #[test_case]           - no
#   #[cfg(test)]           - no  (no `test` after `#\[`)
#
# Breakdown:
#   ^#\[           - starts with `#[`
#   \s*            - optional whitespace
#   (?:\w+::)*     - zero or more namespaced prefixes (e.g. `tokio::`)
#   test           - literal `test`
#   \b             - word boundary (rejects `test_xxx`)
#   \s*[\]\(]?$    - optionally followed by `]` or `(`, then end of line
TEST_ATTR_RE = re.compile(r"^#\[\s*(?:\w+::)*test\b\s*[\]\(]?$")


# ---------- Git helpers ---------------------------------------------------------

def _run(cmd: str, timeout: int = 30) -> str:
    """Run a git command, return stdout. Errors return empty string."""
    try:
        out = subprocess.check_output(
            cmd,
            shell=True,
            cwd=REPO,
            text=True,
            stderr=subprocess.DEVNULL,
            timeout=timeout,
        )
        return out
    except subprocess.CalledProcessError:
        return ""
    except subprocess.TimeoutExpired:
        return ""


def _get_all_commits() -> List[tuple]:
    """Return [(date_str, commit, parent), ...] oldest first."""
    # Use a placeholder (ZZZ) for the format separators so the shell doesn't
    # expand ``%H`` as a variable. The result is then split on ZZZ.
    fmt = "git log --reverse --format='%aI%x09%H%x09%P'"
    raw = _run(fmt)
    result: List[tuple] = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        tokens = line.split("\t")
        # tokens: [iso_date, commit, parent?, parent2?, ...]
        if len(tokens) < 2:
            continue
        date_str = tokens[0][:10]
        commit = tokens[1]
        parent = tokens[2] if len(tokens) >= 3 else ""
        result.append((date_str, commit, parent))
    return result


def _empty_tree_hash() -> str:
    return _run("git hash-object -t tree /dev/null").strip()


def _list_files(commit: str) -> List[str]:
    raw = _run(f"git ls-tree -r --name-only {commit}")
    if not raw:
        return []
    return [f for f in raw.splitlines() if f.strip()]


def _show_file(commit: str, path: str, timeout: int = 10) -> str:
    # Quote the path to be safe with spaces / special chars.
    safe = path.replace('"', '\\"')
    return _run(f'git show "{commit}:{safe}"', timeout=timeout)


def _diff_numstat(parent: str, commit: str, empty_tree: str, timeout: int = 60) -> str:
    if not parent:
        # Root commit: diff against the empty tree.
        return _run(f"git diff {empty_tree} {commit} --numstat", timeout=timeout)
    return _run(f"git diff {parent} {commit} --numstat", timeout=timeout)


# ---------- Classification -----------------------------------------------------

def _classify(path: str) -> str | None:
    """Return 'code', 'doc', or None for excluded."""
    for prefix in EXCLUDED_PATH_PREFIXES:
        if path.startswith(prefix):
            return None
    base = path.rsplit("/", 1)[-1]
    base_lc = base.lower()
    if base_lc in EXCLUDED_FILENAMES:
        return None
    for suf in EXCLUDED_SUFFIXES:
        if base_lc.endswith(suf):
            return None
    if "." not in base:
        return None
    ext = "." + base.rsplit(".", 1)[-1].lower()
    if ext in CODE_EXTS:
        return "code"
    if ext in DOC_EXTS:
        return "doc"
    # Anything else (including all excluded extensions) is dropped.
    return None


# ---------- Counting ------------------------------------------------------------

def _count_loc_and_tests(content: str) -> tuple:
    """
    Return (loc, test_count) for the given file content.

    LOC rules (per spec, mirrors the original script):
      - skip blank lines
      - skip lines that start with ``//`` (after stripping)
      - skip lines inside ``/* ... */`` block comments
      - do NOT handle inline ``//`` or string literals
    Test rules (Rust only — caller decides whether to call this):
      - line is a test attribute if TEST_ATTR_RE matches.
    """
    loc = 0
    tests = 0
    in_block = False
    for line in content.splitlines():
        s = line.strip()
        if not s:
            continue
        if not in_block:
            if s.startswith("/*"):
                # Block comment: may close on same line.
                after_open = s[s.index("/*") + 2:]
                if "*/" in after_open:
                    continue
                in_block = True
                continue
            if s.startswith("//"):
                continue
            loc += 1
            if TEST_ATTR_RE.match(s):
                tests += 1
        else:
            if "*/" in s:
                in_block = False
            # do not count either way
    return loc, tests


def _snapshot(commit: str) -> Dict[str, int]:
    """Snapshot stats for a single commit. All-zero dict if no files."""
    code_files: List[str] = []
    doc_files: List[str] = []
    for f in _list_files(commit):
        kind = _classify(f)
        if kind == "code":
            code_files.append(f)
        elif kind == "doc":
            doc_files.append(f)

    total_loc = 0
    test_cases = 0
    for f in code_files:
        content = _show_file(commit, f)
        if not content:
            continue
        loc, tests = _count_loc_and_tests(content)
        total_loc += loc
        if f.endswith(".rs"):
            test_cases += tests

    doc_lines = 0
    for f in doc_files:
        content = _show_file(commit, f)
        if not content:
            continue
        doc_lines += len(content.splitlines())

    return {
        "code_total_loc": total_loc,
        "test_cases": test_cases,
        "code_files": len(code_files),
        "doc_total_loc": doc_lines,
        "doc_files": len(doc_files),
    }


def _iter_numstat(raw: str):
    """Yield (added, removed, path) tuples, skipping binary / unparseable lines.

    Handles the rename form ``"{old => new}"`` / ``"old => new"`` by using the
    new path on the right-hand side.
    """
    if not raw:
        return
    for line in raw.splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        added_s, removed_s = parts[0], parts[1]
        if added_s == "-" or removed_s == "-":
            # binary file
            continue
        try:
            added = int(added_s)
            removed = int(removed_s)
        except ValueError:
            continue
        path = "\t".join(parts[2:])
        if " => " in path:
            # Rename form. Take the new path.
            new_part = path.rsplit(" => ", 1)[1]
            # Strip trailing similarity score / closing brace, e.g. "}", "]"
            while new_part and new_part[-1] in "}]":
                new_part = new_part[:-1]
            path = new_part
        yield added, removed, path


# ---------- Main collection ----------------------------------------------------

def _empty_dict() -> Dict[str, list]:
    return {
        "dates": [],
        "code_total_loc": [],
        "code_changed_cum": [],
        "doc_total_loc": [],
        "code_files": [],
        "doc_files": [],
        "test_cases": [],
    }


def _collect() -> Dict[str, list]:
    commits = _get_all_commits()
    if not commits:
        return _empty_dict()

    empty_tree = _empty_tree_hash()

    # Group commits by day (oldest first within a day).
    by_day: Dict[str, List[tuple]] = defaultdict(list)
    for date_str, commit, parent in commits:
        by_day[date_str].append((commit, parent))

    oldest_date = commits[0][0]
    today = datetime.now().strftime("%Y-%m-%d")
    end_date = max(oldest_date, today)  # never end before the first commit

    # Pre-compute per-day code diff (sum of |+|+|-| across that day's commits).
    day_code_changed: Dict[str, int] = {}
    for date_str, day_commits in by_day.items():
        total = 0
        for commit, parent in day_commits:
            out = _diff_numstat(parent, commit, empty_tree)
            for added, removed, path in _iter_numstat(out):
                if _classify(path) == "code":
                    total += abs(added) + abs(removed)
        day_code_changed[date_str] = total

    # Walk calendar day by day, forward-fill snapshots, accumulate running total.
    dates: List[str] = []
    code_total_loc: List[int] = []
    code_changed_cum: List[int] = []
    doc_total_loc: List[int] = []
    code_files: List[int] = []
    doc_files: List[int] = []
    test_cases: List[int] = []

    cum_code = 0
    last_snap: Dict[str, int] | None = None

    dt = datetime.strptime(oldest_date, "%Y-%m-%d")
    end_dt = datetime.strptime(end_date, "%Y-%m-%d")
    while dt <= end_dt:
        day = dt.strftime("%Y-%m-%d")
        dates.append(day)

        if day in by_day:
            # Snapshot from the EARLIEST commit of this day.
            earliest_commit = by_day[day][0][0]
            last_snap = _snapshot(earliest_commit)

        if last_snap is None:
            # Shouldn't happen (oldest_date came from a commit), but stay safe.
            last_snap = {
                "code_total_loc": 0, "test_cases": 0, "code_files": 0,
                "doc_total_loc": 0, "doc_files": 0,
            }

        cum_code += day_code_changed.get(day, 0)

        code_total_loc.append(last_snap["code_total_loc"])
        code_changed_cum.append(cum_code)
        doc_total_loc.append(last_snap["doc_total_loc"])
        code_files.append(last_snap["code_files"])
        doc_files.append(last_snap["doc_files"])
        test_cases.append(last_snap["test_cases"])

        dt += timedelta(days=1)

    return {
        "dates": dates,
        "code_total_loc": code_total_loc,
        "code_changed_cum": code_changed_cum,
        "doc_total_loc": doc_total_loc,
        "code_files": code_files,
        "doc_files": doc_files,
        "test_cases": test_cases,
    }


def get_data() -> Dict[str, list]:
    """Collect fresh statistics and return the result dict."""
    return _collect()


if __name__ == "__main__":
    data = get_data()
    n = len(data["dates"])
    print(f"Collected {n} days ({data['dates'][0]} -> {data['dates'][-1]})")
    if n:
        i0, iN = 0, n - 1
        print(
            f"  [{i0:>3}] {data['dates'][i0]}: "
            f"code_loc={data['code_total_loc'][i0]} "
            f"cum={data['code_changed_cum'][i0]} "
            f"doc_loc={data['doc_total_loc'][i0]} "
            f"code_files={data['code_files'][i0]} "
            f"doc_files={data['doc_files'][i0]} "
            f"tests={data['test_cases'][i0]}"
        )
        print(
            f"  [{iN:>3}] {data['dates'][iN]}: "
            f"code_loc={data['code_total_loc'][iN]} "
            f"cum={data['code_changed_cum'][iN]} "
            f"doc_loc={data['doc_total_loc'][iN]} "
            f"code_files={data['code_files'][iN]} "
            f"doc_files={data['doc_files'][iN]} "
            f"tests={data['test_cases'][iN]}"
        )
