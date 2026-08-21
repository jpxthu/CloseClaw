#!/usr/bin/env python3
"""One-pass weekly additions/deletions collector (numstat, code files only).

Writes two files in one git-log run:
  data/weekly_add_del.jsonl  {week, add, del}      for the code-frequency chart
  data/weekly_commits.jsonl  {week, commits}       for validation
Week = ISO week (Monday start). Run as a script; importing it is a no-op.
"""
import json
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timedelta
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.resolve()))
from collect_code_stats import classify  # shared code/doc classification

REPO = Path(__file__).resolve().parents[2]


def collect_weekly() -> tuple:
    """Run one ``git log --numstat`` pass and bucket +/- per ISO week.

    Returns ``(week_add, week_del, week_commits)`` dicts keyed by the week's
    Monday date. Only files classified as ``code`` are counted.
    """
    proc = subprocess.run(
        ["git", "log", "--no-renames", "--numstat",
         "--date=short", "--format=@%ad"],
        cwd=REPO, capture_output=True, text=True, timeout=600,
    )

    week_add = defaultdict(int)
    week_del = defaultdict(int)
    week_commits = defaultdict(int)
    cur = None

    for line in proc.stdout.splitlines():
        if line.startswith("@"):
            d = datetime.strptime(line[1:], "%Y-%m-%d")
            wk = d - timedelta(days=d.weekday())
            week_commits[wk] += 1
            cur = wk
        elif line.strip() and cur is not None:
            parts = line.split("\t")
            if len(parts) == 3:
                a, r, path = parts
                if a != "-" and classify(path) == "code":
                    week_add[cur] += int(a)
                    week_del[cur] += int(r)
    return week_add, week_del, week_commits


def main() -> None:
    """Collect weekly stats and write the two JSONL outputs under data/."""
    week_add, week_del, week_commits = collect_weekly()

    out = Path(__file__).parent / "data"
    out.mkdir(exist_ok=True)
    with open(out / "weekly_add_del.jsonl", "w") as f:
        weeks = sorted(set(week_add) | set(week_del))
        for wk in weeks:
            rec = {
                "week": wk.strftime("%Y-%m-%d"),
                "add": week_add[wk],
                "del": week_del[wk],
            }
            f.write(json.dumps(rec) + "\n")
    with open(out / "weekly_commits.jsonl", "w") as f:
        for wk in sorted(week_commits):
            rec = {"week": wk.strftime("%Y-%m-%d"),
                   "commits": week_commits[wk]}
            f.write(json.dumps(rec) + "\n")
    print(f"weeks={len(week_commits)} "
          f"total_commits={sum(week_commits.values())}")


if __name__ == "__main__":
    main()
