#!/usr/bin/env python3
"""
Collect daily code statistics from CloseClaw master branch.

Usage:
    python3 collect_code_stats.py [--verbose]
    python3 collect_code_stats.py --help

Outputs one JSONL record per calendar day (including zero-activity days).
Each record contains: date, commit, rs_files, total_loc, test_cases.
"""

import subprocess, json, sys, os
from datetime import datetime, timedelta
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent.resolve()
REPO = SCRIPT_DIR.parent  # closeclaw-test repo root
DATA_DIR = SCRIPT_DIR / "data"

# Ensure data directory exists
DATA_DIR.mkdir(exist_ok=True)

def parse_args():
    help_flag = "--help" in sys.argv or "-h" in sys.argv
    verbose = "--verbose" in sys.argv or "-v" in sys.argv
    return help_flag, verbose

HELP_TEXT = f"""CloseClaw Daily Code Statistics Collector

Usage:
    python3 {os.path.basename(__file__)} [--verbose]
    python3 {os.path.basename(__file__)} --help

Description:
    Walk master branch day by day from first commit to today.
    For each calendar day, snapshot the LAST commit on or before that day.
    Count: Rust source files (.rs), total LOC, #[test] functions.

Output:
    Writes to: {REPO}/scripts/daily_stats.jsonl
    Format: one JSON object per line

      {{"date": "2026-03-21", "commit": "42ea4d78c81a",
        "rs_files": 32, "total_loc": 4786, "test_cases": 50}}

Options:
    --verbose, -v   Print per-day processing details to stderr
    --help, -h       Show this help message

Examples:
    # Quiet run (default)
    python3 collect_code_stats.py

    # Verbose run
    python3 collect_code_stats.py --verbose

Notes:
    - LOC excludes empty lines and // comments; handles /* */ block comments.
    - grep exit code 1 (no match) is handled gracefully.
    - Running time ~5-10 min for 336 commits.
"""

def run(cmd, timeout=30):
    try:
        return subprocess.check_output(
            cmd, shell=True, cwd=REPO, text=True,
            stderr=subprocess.DEVNULL, timeout=timeout
        ).strip()
    except subprocess.CalledProcessError as e:
        if e.returncode == 1 and "grep" in cmd:
            return ""
        return ""

def log(msg):
    if _verbose:
        print(msg, file=sys.stderr)

_verbose = False

# ── Git helpers ─────────────────────────────────────────────────────────────────

def get_commits():
    """Newest-first list of (date_YYYY_MM_DD, commit_hash)."""
    out = run("git log --format='%aI %H'")
    result = []
    for line in out.splitlines():
        if not line.strip(): continue
        tokens = line.strip().split()
        result.append((tokens[0][:10], tokens[-1]))
    return result

def latest_commit_before(day, commits_newest_first):
    """
    Return the NEWEST commit with date <= day.
    Iterates oldest-first (reversed) so first match is the newest valid.
    """
    best = None
    for d, c in reversed(commits_newest_first):
        if d <= day:
            best = c
        elif d > day:
            break
    return best

# ── Counting ────────────────────────────────────────────────────────────────────

def count_stats(commit):
    """Count .rs files, total LOC, #[test] functions at given commit."""
    files_out = run(f"git ls-tree -r --name-only {commit} | grep '\\.rs$'")
    if not files_out:
        raw = run(f"git ls-tree -r --name-only {commit}")
        rs = [f for f in raw.splitlines() if f.strip().endswith(".rs")] if raw else []
    else:
        rs = [f for f in files_out.splitlines() if f.strip()] if files_out else []

    total_loc = 0
    test_count = 0
    for f in rs:
        fc = run(f"git show {commit}:{f}", timeout=10)
        if not fc:
            continue
        in_block = False
        for line in fc.splitlines():
            s = line.strip()
            if not s:
                continue
            if not in_block:
                if s.startswith("/*"):
                    if "*/" in s[s.index("/*")+2:]:
                        continue
                    else:
                        in_block = True
                        continue
                elif s.startswith("//"):
                    continue
                else:
                    if s.startswith("#[test") or s.startswith("#[tokio::test"):
                        test_count += 1
                    total_loc += 1
            else:
                if "*/" in s:
                    in_block = False
                continue
    return {"rs_files": len(rs), "total_loc": total_loc, "test_cases": test_count}

def iter_days(start, end):
    dt = datetime.strptime(start, "%Y-%m-%d")
    ed = datetime.strptime(end, "%Y-%m-%d")
    while dt <= ed:
        yield dt.strftime("%Y-%m-%d")
        dt += timedelta(days=1)

# ── Main ─────────────────────────────────────────────────────────────────────────

def main():
    global _verbose
    help_flag, _verbose = parse_args()

    if help_flag:
        print(HELP_TEXT)
        return

    commits = get_commits()
    if not commits:
        print("ERROR: no commits found in repository", file=sys.stderr)
        return

    oldest = commits[-1][0]
    newest = commits[0][0]
    log(f"Commits: {len(commits)}, range {oldest} → {newest}")

    jsonl_path = DATA_DIR / "daily_stats.jsonl"
    results = []

    for day in iter_days(oldest, newest):
        commit = latest_commit_before(day, commits)
        if commit is None:
            results.append({"date": day, "commit": None,
                            "rs_files": 0, "total_loc": 0, "test_cases": 0})
            continue

        stats = count_stats(commit)
        log(f"  {day}: {commit[:8]} → {stats['rs_files']} files, "
            f"{stats['total_loc']} LOC, {stats['test_cases']} tests")

        results.append({"date": day, "commit": commit[:12],
                        "rs_files": stats["rs_files"],
                        "total_loc": stats["total_loc"],
                        "test_cases": stats["test_cases"]})

    with open(jsonl_path, "w") as f:
        for r in results:
            f.write(json.dumps(r) + "\n")

    print(f"Collected {len(results)} daily records → {jsonl_path}")
    if not _verbose:
        print("  (re-run with --verbose for per-day details)")

if __name__ == "__main__":
    main()