#!/usr/bin/env python3
"""
Collect real UT coverage from cargo-llvm-cov and append to history.

Usage:
    python3 collect_coverage.py [--verbose]
    python3 collect_coverage.py --help

Appends one record to scripts/coverage_history.jsonl:
  {"date": "2026-05-02", "commit": "abc123def", "avg_coverage": 83.35, "max_coverage": 100.0}

Requires: cargo-llvm-cov, LLVM tools (set via env vars).
"""

import subprocess, json, sys, os, re
from datetime import datetime
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent.resolve()
REPO = SCRIPT_DIR.parent
DATA_DIR = SCRIPT_DIR / "data"
HISTORY_FILE = DATA_DIR / "coverage_history.jsonl"

# Ensure data directory exists
DATA_DIR.mkdir(exist_ok=True)

LLVM_ENV = {
    "LLVM_CONFIG": "/home/linuxbrew/.linuxbrew/bin/llvm-config",
    "LLVM_COV": "/home/linuxbrew/.linuxbrew/bin/llvm-cov",
    "LLVM_PROFDATA": "/home/linuxbrew/.linuxbrew/bin/llvm-profdata",
}

def parse_args():
    help_flag = "--help" in sys.argv or "-h" in sys.argv
    verbose = "--verbose" in sys.argv or "-v" in sys.argv
    return help_flag, verbose

HELP_TEXT = f"""CloseClaw Real Coverage Collector

Usage:
    python3 {{os.path.basename(__file__)}} [--verbose]
    python3 {{os.path.basename(__file__)}} --help

Description:
    Run cargo llvm-cov on current HEAD, parse summary output.
    Extract: average (TOTAL) coverage, max per-file coverage.
    Append one record to coverage_history.jsonl.

Output:
    {HISTORY_FILE}

Options:
    --verbose, -v   Print llvm-cov output and parsed values
    --help, -h       Show this help message
"""

_verbose = False

def log(msg):
    if _verbose:
        print(msg, file=sys.stderr)


def get_current_commit():
    """Get short commit hash of HEAD."""
    try:
        out = subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=REPO, text=True, stderr=subprocess.DEVNULL
        ).strip()
        return out
    except subprocess.CalledProcessError:
        return "unknown"


def run_llvm_cov():
    """Run cargo llvm-cov --summary-only and return stdout."""
    env = os.environ.copy()
    env.update(LLVM_ENV)
    try:
        result = subprocess.run(
            ["cargo", "llvm-cov", "--package", "closeclaw", "--lib", "--summary-only"],
            cwd=REPO, text=True, capture_output=True, env=env, timeout=600
        )
        log(result.stderr)  # compilation info goes to stderr
        return result.stdout
    except subprocess.TimeoutExpired:
        print("ERROR: cargo llvm-cov timed out (600s)", file=sys.stderr)
        return None


def parse_coverage(output):
    """
    Parse llvm-cov summary output.
    Returns avg_coverage (TOTAL line) or None on failure.

    Output format (whitespace-separated):
      Filename    Regions Missed  Covered  Coverage  Functions Missed  Covered  Coverage  Lines Missed  Covered  Coverage  Branches ...
      TOTAL       31217           5198     83.35%    3005              589      80.40%    20751         3265     84.27%    0           0         -
    """
    if not output:
        return None

    for line in output.splitlines():
        line = line.strip()
        if not line or line.startswith("---") or line.startswith("Filename"):
            continue

        parts = line.split()
        if len(parts) < 4:
            continue

        if parts[0] == "TOTAL":
            for p in parts:
                if p.endswith("%"):
                    try:
                        return float(p.rstrip("%"))
                    except ValueError:
                        continue

    print("ERROR: could not find TOTAL line in llvm-cov output", file=sys.stderr)
    return None


def load_existing_dates():
    """Load dates already in history to avoid duplicates."""
    dates = set()
    if HISTORY_FILE.exists():
        with open(HISTORY_FILE) as f:
            for line in f:
                try:
                    rec = json.loads(line)
                    dates.add(rec.get("date"))
                except json.JSONDecodeError:
                    pass
    return dates


def main():
    global _verbose
    help_flag, _verbose = parse_args()

    if help_flag:
        print(HELP_TEXT)
        return

    today = datetime.now().strftime("%Y-%m-%d")
    existing = load_existing_dates()

    if today in existing:
        print(f"Coverage for {today} already recorded. Skipping.")
        print(f"  Delete the entry from {HISTORY_FILE} to re-collect.")
        return

    commit = get_current_commit()
    print(f"Running cargo llvm-cov on {commit}...")

    output = run_llvm_cov()
    if _verbose and output:
        log("--- llvm-cov output ---")
        log(output)
        log("--- end ---")

    result = parse_coverage(output)
    if result is None:
        print("FAILED: could not parse coverage", file=sys.stderr)
        sys.exit(1)

    avg_cov = result

    record = {
        "date": today,
        "commit": commit,
        "avg_coverage": avg_cov,
    }

    with open(HISTORY_FILE, "a") as f:
        f.write(json.dumps(record) + "\n")

    print(f"Recorded: avg={avg_cov}% → {HISTORY_FILE}")


if __name__ == "__main__":
    main()
