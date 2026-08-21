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

Incremental cache
-----------------
When ``get_data(use_cache=True)`` (the default) is called and ``cache.json``
exists at ``scripts/code-stats/data/cache.json``, the collector reuses the
previous run's per-day stats and only walks commits with author date on or
after the cache's last date. Use ``--no-cache`` to force a full re-collect.

Output
------
``get_data()`` returns a dict ready to be consumed by ``draw_stats.py``:

    {
        "dates":           ["2026-03-21", ...],
        "code_total_loc":  [...],   # curve 1-1 (snapshot)
        "code_changed_cum":[...],   # curve 1-2 (running total of |diff|)
        "doc_total_loc":   [...],   # curve 2
        "code_files":      [...],   # curve 3-1
        "doc_files":       [...],   # curve 3-2
        "test_cases":      [...],   # curve 4
        "test_loc":        [...],   # test code LOC
    }

Linear-master assumption
------------------------
We assume ``master`` is linear and new commits only appear at the tip (no
rebases / force-pushes). The cache records ``HEAD`` at generation time;
``_cache_head_is_ancestor`` checks on load that the recorded hash is still
reachable from current ``HEAD``. If not (rebase / force-push / GC'd object),
the collector warns on stderr and falls back to a full re-collect — no
manual ``--no-cache`` required.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timedelta
from pathlib import Path
from typing import Dict, List

sys.path.insert(0, str(Path(__file__).parent.resolve()))
from cache import SCHEMA_VERSION, load_cache, save_cache

SCRIPT_DIR = Path(__file__).parent.resolve()
REPO = SCRIPT_DIR.parent.parent  # closeclaw repo root
CACHE_PATH = SCRIPT_DIR / "data" / "cache.json"

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
    """Return the hash of the empty git tree (4b825dc6... convention)."""
    return _run("git hash-object -t tree /dev/null").strip()


def _list_files(commit: str) -> List[str]:
    """List all file paths at ``commit`` (recursive). Empty list on failure."""
    raw = _run(f"git ls-tree -r --name-only {commit}")
    if not raw:
        return []
    return [f for f in raw.splitlines() if f.strip()]


def _show_file(commit: str, path: str, timeout: int = 10) -> str:
    """Return file ``path`` content at ``commit``. Empty string on failure."""
    # Quote the path to be safe with spaces / special chars.
    safe = path.replace('"', '\\"')
    return _run(f'git show "{commit}:{safe}"', timeout=timeout)


def _diff_numstat(parent: str, commit: str, empty_tree: str, timeout: int = 60) -> str:
    """Return ``git diff <parent> <commit> --numstat`` (vs empty tree for root)."""
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


# Public alias so sibling scripts (collect_weekly_add_del) can reuse the
# classification without importing a private name.
classify = _classify


# ---------- Test file detection via #[path] convention --------------------------

# Files ending with ``_tests.rs`` that are NOT in the ``tests/`` directory.
# These are test module files included via ``#[path = "xxx_tests.rs"]`` in
# the parent module.  They contain real test code but have no inner
# ``#[cfg(test)]`` gate — the gate lives in the referencing parent file.
# Convention: suffix ``_tests.rs``, located next to the module that includes them.
def _is_path_included_test(path: str) -> bool:
    """Return True for *_tests.rs files outside the tests/ directory."""
    return path.endswith("_tests.rs") and not path.startswith("tests/")


# ---------- Counting ------------------------------------------------------------

# Matches ``#[cfg(test)]`` (Rust test module gate).
_CFG_TEST_RE = re.compile(r"^#\[\s*cfg\s*\(\s*test\s*\)\s*\]$")


def _update_cfg_test_state(
    s: str, in_cfg_test: bool, brace_depth: int, entered: bool
) -> tuple:
    """Update Rust ``#[cfg(test)]`` block-tracking state for one line.

    Returns ``(in_cfg_test, brace_depth, entered, is_inside)`` where
    ``is_inside`` is True iff the line falls inside an open ``#[cfg(test)]``
    block (and therefore should be counted as test LOC).
    """
    if in_cfg_test:
        prev_depth = brace_depth
        brace_depth += s.count('{') - s.count('}')
        if not entered and brace_depth > prev_depth and brace_depth >= 1:
            entered = True
        if entered and brace_depth <= 0:
            in_cfg_test = False
        return in_cfg_test, brace_depth, entered, True
    if _CFG_TEST_RE.match(s):
        return True, 0, False, False
    return in_cfg_test, brace_depth, entered, False


def _count_loc_and_tests(content: str) -> tuple:
    """Return (loc, test_count, test_loc).

    LOC skips blank lines, ``//`` lines, and lines inside ``/* */`` blocks.
    Tests count ``#[test]`` attributes; ``test_loc`` counts lines inside
    ``#[cfg(test)]`` blocks (Rust test modules).
    """
    loc = 0
    tests = 0
    test_loc = 0
    in_block = False
    in_cfg_test = False
    brace_depth = 0
    entered = False
    for line in content.splitlines():
        s = line.strip()
        if not s:
            continue
        if not in_block:
            if s.startswith("/*"):
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
            in_cfg_test, brace_depth, entered, is_inside = _update_cfg_test_state(
                s, in_cfg_test, brace_depth, entered
            )
            if is_inside:
                test_loc += 1
        else:
            if "*/" in s:
                in_block = False
    return loc, tests, test_loc


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
    test_loc = 0
    for f in code_files:
        content = _show_file(commit, f)
        if not content:
            continue
        loc, tests, tloc = _count_loc_and_tests(content)
        total_loc += loc
        if f.endswith(".rs"):
            test_cases += tests
            if f.startswith("tests/"):
                # Integration test files: entire file is test code.
                test_loc += loc
            elif _is_path_included_test(f):
                # Test module files included via #[path = "xxx_tests.rs"]:
                # entire file is test code (no inner #[cfg(test)] wrapper).
                test_loc += loc
            else:
                test_loc += tloc

    doc_lines = 0
    for f in doc_files:
        content = _show_file(commit, f)
        if not content:
            continue
        doc_lines += len(content.splitlines())

    return {
        "code_total_loc": total_loc,
        "test_cases": test_cases,
        "test_loc": test_loc,
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
    """Return a fresh empty per-day columns dict (all lists)."""
    return {
        "dates": [],
        "code_total_loc": [],
        "code_changed_cum": [],
        "doc_total_loc": [],
        "code_files": [],
        "doc_files": [],
        "test_cases": [],
        "test_loc": [],
    }


def _group_by_day(commits: List[tuple]) -> Dict[str, List[tuple]]:
    """Group [(date_str, commit, parent), ...] by date_str (insertion order)."""
    by_day: Dict[str, List[tuple]] = defaultdict(list)
    for date_str, commit, parent in commits:
        by_day[date_str].append((commit, parent))
    return by_day


def _compute_day_churn(
    by_day: Dict[str, List[tuple]], empty_tree: str
) -> Dict[str, int]:
    """Sum |+| + |-| across each day's commits for code files only."""
    day_code_changed: Dict[str, int] = {}
    for date_str, day_commits in by_day.items():
        total = 0
        for commit, parent in day_commits:
            out = _diff_numstat(parent, commit, empty_tree)
            for added, removed, path in _iter_numstat(out):
                if _classify(path) == "code":
                    total += abs(added) + abs(removed)
        day_code_changed[date_str] = total
    return day_code_changed


_ZERO_SNAP = {
    "code_total_loc": 0, "test_cases": 0, "test_loc": 0,
    "code_files": 0, "doc_total_loc": 0, "doc_files": 0,
}


def _emit_day(
    day: str,
    by_day: Dict[str, List[tuple]],
    day_code_changed: Dict[str, int],
    last_snap: Dict[str, int] | None,
    cum_code: int,
) -> tuple:
    """Process one calendar day; return (new_last_snap, new_cum_code, day_record).

    ``day_record`` is the ``days[day]`` entry: commit + changed + snap.
    The returned ``snap`` is a fresh ``dict`` copy (not aliased with the
    caller-held ``last_snap``) so cache snapshots cannot be mutated through
    either reference later. Safety-zero-fills ``last_snap`` if it is still
    None on the first iteration.
    """
    if day in by_day:
        earliest_commit = by_day[day][0][0]
        snap = _snapshot(earliest_commit)
        day_record = {
            "commit": earliest_commit,
            "changed": day_code_changed.get(day, 0),
            "snap": dict(snap),  # defensive copy: isolate days[] from last_snap
        }
        last_snap = snap
    else:
        day_record = {"commit": "", "changed": 0, "snap": None}

    if last_snap is None:
        # Shouldn't happen (oldest_date came from a commit), but stay safe.
        last_snap = dict(_ZERO_SNAP)

    cum_code += day_code_changed.get(day, 0)
    return last_snap, cum_code, day_record


def _append_day_to_columns(buckets: dict, day: str, last_snap: dict, cum_code: int) -> None:
    """Push one day of stats into the per-key buckets (in place)."""
    buckets["dates"].append(day)
    buckets["code_total_loc"].append(last_snap["code_total_loc"])
    buckets["code_changed_cum"].append(cum_code)
    buckets["doc_total_loc"].append(last_snap["doc_total_loc"])
    buckets["code_files"].append(last_snap["code_files"])
    buckets["doc_files"].append(last_snap["doc_files"])
    buckets["test_cases"].append(last_snap["test_cases"])
    buckets["test_loc"].append(last_snap["test_loc"])


def _walk_calendar(
    oldest_date: str,
    end_date: str,
    by_day: Dict[str, List[tuple]],
    day_code_changed: Dict[str, int],
    initial_cum: int,
    initial_snap: Dict[str, int] | None = None,
) -> tuple:
    """Walk [oldest_date, end_date] day by day, forward-fill snapshots.

    Returns ``(columns_dict, days_dict)``. ``days_dict[k]`` is the anchor
    commit + changed count + snap for that calendar day (snap is ``None``
    if the day had no commit). ``initial_snap`` lets incremental callers
    seed the forward-fill from the cache's last known snapshot, so multi-day
    gaps don't drop to all-zero before the first new-commit day.
    """
    columns = _empty_dict()
    days: Dict[str, dict] = {}

    cum_code = initial_cum
    last_snap: Dict[str, int] | None = (
        dict(initial_snap) if initial_snap is not None else None
    )
    dt = datetime.strptime(oldest_date, "%Y-%m-%d")
    end_dt = datetime.strptime(end_date, "%Y-%m-%d")

    while dt <= end_dt:
        day = dt.strftime("%Y-%m-%d")
        last_snap, cum_code, days[day] = _emit_day(
            day, by_day, day_code_changed, last_snap, cum_code,
        )
        _append_day_to_columns(columns, day, last_snap, cum_code)
        dt += timedelta(days=1)

    return columns, days


def _collect(
    since_date: str | None = None,
    initial_cum: int = 0,
    initial_snap: Dict[str, int] | None = None,
) -> dict:
    """Walk git log and build the per-day output for [since_date, today].

    Args:
        since_date: earliest YYYY-MM-DD to include. ``None`` = use the oldest
            commit's date (full walk).
        initial_cum: starting value for cumulative code churn. Used by
            incremental collection to continue from the previous cum_code.
        initial_snap: starting snapshot for the forward-fill chain. Used by
            incremental collection to seed from the cache's last non-None
            snap so multi-day gaps don't collapse to zero.

    Returns:
        ``{"columns": {...}, "days": {...}}``. ``columns`` has the same
        shape as ``get_data()``'s return value. ``days`` maps YYYY-MM-DD to
        ``{"commit": str, "changed": int, "snap": dict|None}`` for each
        calendar day in the walked range.
    """
    all_commits = _get_all_commits()
    if not all_commits:
        return {"columns": _empty_dict(), "days": {}}

    empty_tree = _empty_tree_hash()

    filtered = (
        all_commits if since_date is None
        else [c for c in all_commits if c[0] >= since_date]
    )
    by_day = _group_by_day(filtered)

    oldest_date = since_date if since_date else all_commits[0][0]
    today = datetime.now().strftime("%Y-%m-%d")
    end_date = max(oldest_date, today)  # never end before the first commit

    day_code_changed = _compute_day_churn(by_day, empty_tree)
    columns, days = _walk_calendar(
        oldest_date, end_date, by_day, day_code_changed,
        initial_cum, initial_snap,
    )
    return {"columns": columns, "days": days}


def _last_non_none_snap(cache_days: Dict[str, dict]) -> Dict[str, int] | None:
    """Return the latest calendar day's non-None snap from ``cache_days``.

    Walks dates in descending order until a snap is found. Returns ``None``
    if every day in the cache has ``snap=None`` (extremely edge: cache
    contains only no-commit days).
    """
    for day in sorted(cache_days.keys(), reverse=True):
        snap = cache_days[day].get("snap")
        if snap is not None:
            return snap
    return None


def _incremental_pre_state(cache: dict, today: str) -> dict:
    """Pick pre-collect cache state for incremental, depending on today's date.

    Returns ``{"pre_cols", "pre_days", "since_date", "pre_cum", "initial_snap"}``.
    ``initial_snap`` is the last non-None snapshot from the cache so the new
    walk's forward-fill chain doesn't collapse to zero on a multi-day gap.
    """
    cache_cols = cache["columns"]
    cache_days = cache["days"]
    cache_last_date = cache_cols["dates"][-1]
    cum_list = cache_cols["code_changed_cum"]
    if len(cum_list) >= 2:
        pre_cum = cum_list[-2]
    else:
        # Single-day cache: derive the pre-day cum from that day's churn.
        only_day = cache_days.get(cache_last_date, {})
        pre_cum = cum_list[-1] - only_day.get("changed", 0)
    initial_snap = _last_non_none_snap(cache_days)

    if today == cache_last_date:
        return {
            "pre_cols": {k: v[:-1] for k, v in cache_cols.items()},
            "pre_days": {k: v for k, v in cache_days.items() if k != today},
            "since_date": today,
            "pre_cum": pre_cum,
            "initial_snap": initial_snap,
        }
    # today > cache_last_date: cache covers up to cache_last_date; we walk
    # from cache_last_date with pre_cum, then drop cache_last_date from new.
    return {
        "pre_cols": cache_cols,
        "pre_days": cache_days,
        "since_date": cache_last_date,
        "pre_cum": pre_cum,
        "initial_snap": initial_snap,
    }


def _warn_fallback(reason: str) -> None:
    """Print a stderr warning that the cache is ignored (full re-collect)."""
    print(
        f"WARNING: {reason}; ignoring cache (full re-collect).",
        file=sys.stderr,
    )


def _collect_incremental(cache: dict) -> dict:
    """Build the full result using a previous cache + new commits only.

    Two cases, depending on whether ``today`` matches the cache's last date:
      - ``today == cache_last_date``: cache is stale on today. Truncate today
        from cache, re-walk today from scratch with ``initial_cum`` set to
        the cum_code at end of yesterday and ``initial_snap`` set to the
        cache's last non-None snapshot.
      - ``today > cache_last_date``: cache is complete up to ``cache_last_date``.
        Walk from ``cache_last_date`` with ``initial_cum`` set to the cum_code
        at end of the day *before* ``cache_last_date``; then drop the first
        day from the new result (which is ``cache_last_date`` itself, already
        in cache). Multi-day gaps are handled correctly because
        ``initial_snap`` seeds the forward-fill from the cache.

    Falls back to a full re-collect (with a stderr warning) on clock skew or
    when same-day late commits made the cached last day stale.

    Returns ``{"columns": ..., "days": ...}``.
    """
    today = datetime.now().strftime("%Y-%m-%d")
    cache_last_date = cache["columns"]["dates"][-1]
    if today < cache_last_date:
        # Clock skew / unusual env. Cache contains future data we can't trust.
        _warn_fallback(
            f"cache.json last date {cache_last_date} is in the future "
            f"(today={today})"
        )
        return _collect()

    state = _incremental_pre_state(cache, today)
    new_result = _collect(
        since_date=state["since_date"],
        initial_cum=state["pre_cum"],
        initial_snap=state["initial_snap"],
    )
    if today > cache_last_date and _cache_day_is_stale(cache, new_result):
        _warn_fallback(
            f"cache.json is stale for {cache_last_date} "
            f"(commits landed after cache was written)"
        )
        return _collect()
    if today > cache_last_date:
        _drop_first_day(new_result, cache_last_date)
    return _merge_pre_and_new(state, new_result)


def _cache_day_is_stale(cache: dict, new_result: dict) -> bool:
    """Detect same-day late commits by comparing the re-walked first day.

    If re-walking ``cache_last_date`` now yields a different cum than the
    cache recorded, new commits landed on that day after the cache was
    written — the cached value is stale and the caller must fall back to
    a full re-collect (dropping the recomputed day would silently lose
    that churn).
    """
    new_cum_first = new_result["columns"]["code_changed_cum"][0]
    return new_cum_first != cache["columns"]["code_changed_cum"][-1]


def _drop_first_day(new_result: dict, cache_last_date: str) -> None:
    """Drop ``cache_last_date`` from the new walk (already covered by cache)."""
    new_result["columns"] = {
        k: v[1:] for k, v in new_result["columns"].items()
    }
    new_result["days"] = {
        k: v for k, v in new_result["days"].items() if k != cache_last_date
    }


def _merge_pre_and_new(state: dict, new_result: dict) -> dict:
    """Concatenate pre-state columns/days with the new walk's result."""
    return {
        "columns": {
            k: state["pre_cols"][k] + new_result["columns"][k]
            for k in state["pre_cols"]
        },
        "days": {**state["pre_days"], **new_result["days"]},
    }


def _cache_head_is_ancestor(cache_head: str) -> bool:
    """Return True iff ``cache_head`` is reachable from current ``HEAD``.

    Uses ``git merge-base --is-ancestor``; first verifies the commit object
    exists locally (so a GC'd / unknown hash returns False cleanly). On
    rebase, force-push, or hash absence, returns False — the caller must
    fall back to a full re-collect to keep ``commit-hash consistency``
    (per owner requirement).
    """
    if not cache_head:
        return False
    try:
        # Existence check first: a missing object shouldn't fail loudly.
        exists_rc = subprocess.run(
            ["git", "cat-file", "-e", cache_head],
            cwd=REPO, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        ).returncode
        if exists_rc != 0:
            return False
        return subprocess.run(
            ["git", "merge-base", "--is-ancestor", cache_head, "HEAD"],
            cwd=REPO, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        ).returncode == 0
    except (OSError, subprocess.SubprocessError):
        # git missing / not executable / timed out — treat as "cannot verify".
        return False


def _build_cache_payload(result: dict) -> dict:
    """Wrap a collect result into the cache file schema."""
    head = _run("git rev-parse --short HEAD").strip()
    return {
        "schema_version": SCHEMA_VERSION,
        "head": head,
        "generated_at": datetime.now().isoformat(timespec="seconds"),
        "columns": result["columns"],
        "days": result["days"],
    }


def _do_collect(use_cache: bool) -> dict:
    """Try cache; fall back to full; save on success. Returns full result.

    Cache is rejected (forced full re-collect) when:
      - load returns ``None`` (corrupt / wrong schema / missing fields)
      - ``cache.head`` is not a current ``HEAD`` ancestor (rebase / force-push)
    """
    cache = load_cache(str(CACHE_PATH)) if use_cache else None
    if cache is None:
        result = _collect()
    elif not _cache_head_is_ancestor(cache.get("head", "")):
        print(
            f"WARNING: cache.json head={cache.get('head', '')!r} is not an "
            f"ancestor of HEAD; ignoring cache (full re-collect).",
            file=sys.stderr,
        )
        result = _collect()
    else:
        result = _collect_incremental(cache)
    if use_cache:
        save_cache(str(CACHE_PATH), _build_cache_payload(result))
    return result


def get_data(use_cache: bool = True) -> Dict[str, list]:
    """Collect daily statistics, returning the public ``columns`` dict.

    ``use_cache=True`` (default) loads ``data/cache.json`` if present and
    incrementally walks only new commits. ``--no-cache`` passes ``False`` to
    force a full re-collect.
    """
    result = _do_collect(use_cache)
    return result["columns"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Collect daily code statistics from git history."
    )
    parser.add_argument(
        "--no-cache", action="store_true",
        help="Ignore cache.json and do a full re-collect.",
    )
    args = parser.parse_args()

    data = get_data(use_cache=not args.no_cache)
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
            f"tests={data['test_cases'][i0]} "
            f"test_loc={data['test_loc'][i0]}"
        )
        print(
            f"  [{iN:>3}] {data['dates'][iN]}: "
            f"code_loc={data['code_total_loc'][iN]} "
            f"cum={data['code_changed_cum'][iN]} "
            f"doc_loc={data['doc_total_loc'][iN]} "
            f"code_files={data['code_files'][iN]} "
            f"doc_files={data['doc_files'][iN]} "
            f"tests={data['test_cases'][iN]} "
            f"test_loc={data['test_loc'][iN]}"
        )
