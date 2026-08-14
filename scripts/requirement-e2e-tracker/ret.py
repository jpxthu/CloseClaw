#!/usr/bin/env python3
"""
Requirement E2E Tracker (ret) – track test-case discovery status per module.

A *module* is the unit of tracking.  A module is defined by its requirement
doc ``docs/requirements/<module>.md`` (``README.md`` / ``STANDARDS.md`` are
not modules), plus every ``.md`` file under ``docs/design/<module>/``.
When any of those docs change, the module's test cases must be rediscovered.

Commands
--------
- ``finished <module>``
    Record that the test cases for *<module>* have been discovered.  The
    recorded commit is ``git merge-base HEAD origin/master``.  Clears any
    existing comment and blocked_reason.

- ``blocked <module> <reason>``
    Mark a module as blocked with a reason.  If the module already has a
    record, the blocked_reason is updated; otherwise a new record is created.

- ``comment <module> <text>``
    Override the comment for a module.  If the module already has a record
    the comment is updated; otherwise a new record is created with an empty
    commit.

- ``check``
    Scan every module (derived from ``docs/requirements/*.md``) and report
    any whose requirement/design docs changed since their last confirmation,
    or that have never been tracked.  Blocked modules that have NOT been
    updated are silently skipped; blocked modules that HAVE been updated are
    auto-unblocked and reported as normal changes.

records.json lives alongside this script.  Each record has the fields:
``module``, ``commit``, ``commit_time``, ``confirmed_time``, ``comment``,
``blocked_reason``.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path
from types import SimpleNamespace
from typing import Dict, List

import click

_SCRIPTS_DIR = Path(__file__).resolve().parent.parent
if str(_SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(_SCRIPTS_DIR))

from _tracker_core import (  # noqa: E402
    collect_md_files,
    commit_committer_date,
    diff_quiet_changed,
    load_records,
    merge_base_commit,
    now_iso,
    run,
    save_records,
    upsert_record,
)

SCRIPT_DIR = Path(__file__).parent.resolve()
REPO_ROOT = Path(
    subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=SCRIPT_DIR,
        text=True,
    ).strip()
)
RECORDS_FILE = SCRIPT_DIR / "records.json"
REQUIREMENTS_DIR = REPO_ROOT / "docs" / "requirements"
DESIGN_DOC_DIR = REPO_ROOT / "docs" / "design"

# Requirement files that are not modules.
EXCLUDED_REQUIREMENTS = frozenset({"README.md", "STANDARDS.md"})

_RECORD_DEFAULTS: Dict[str, str] = {
    "commit": "",
    "commit_time": "",
    "confirmed_time": "",
    "comment": "",
    "blocked_reason": "",
}

# ── helpers ──────────────────────────────────────────────────────────────


def _run(cmd: List[str], **kwargs) -> subprocess.CompletedProcess[str]:
    """Run a command inside the repo root, return CompletedProcess."""
    return run(cmd, REPO_ROOT, **kwargs)


def _commit_committer_date(ref: str = "HEAD") -> str:
    """Return ISO-8601 committer date for *ref*."""
    return commit_committer_date(_run, ref)


def _merge_base_commit() -> str | None:
    """Return the merge-base of HEAD and origin/master, or None on failure."""
    return merge_base_commit(_run)


def _load_records() -> List[Dict[str, str]]:
    return load_records(RECORDS_FILE, {"blocked_reason": ""})


def _save_records(records: List[Dict[str, str]]) -> None:
    save_records(RECORDS_FILE, records, "module")


def _now_iso() -> str:
    return now_iso()


def _upsert_record(
    records: List[Dict[str, str]], module: str, **fields: str
) -> List[Dict[str, str]]:
    """Find or create a record for *module*, then update it with *fields*."""
    return upsert_record(records, "module", module, _RECORD_DEFAULTS, **fields)


def _discover_modules() -> List[str]:
    """Return module names derived from ``docs/requirements/*.md`` filenames."""
    if not REQUIREMENTS_DIR.exists():
        return []
    modules: List[str] = []
    for p in sorted(REQUIREMENTS_DIR.glob("*.md")):
        if p.is_file() and p.name not in EXCLUDED_REQUIREMENTS:
            modules.append(p.stem)
    return modules


def _module_docs(module: str) -> List[Path]:
    """Return the repo-relative ``.md`` paths tracked for *module*.

    The tracked set is ``docs/requirements/<module>.md`` plus every ``.md``
    file under ``docs/design/<module>/`` (recursively).
    """
    docs: List[Path] = []
    req = REQUIREMENTS_DIR / f"{module}.md"
    if req.is_file():
        docs.append(Path(str(req.relative_to(REPO_ROOT))))
    design_dir = DESIGN_DOC_DIR / module
    if design_dir.is_dir():
        docs.extend(collect_md_files(REPO_ROOT, design_dir))
    return docs


def _module_changed(commit: str, docs: List[Path]) -> bool:
    """Return True when any tracked doc changed between *commit* and HEAD."""
    return diff_quiet_changed(_run, commit, [str(d) for d in docs])


# ── sub-commands ─────────────────────────────────────────────────────────


def cmd_finished(args: SimpleNamespace) -> int:
    module = args.module

    docs = _module_docs(module)
    if not docs:
        print(
            f"Error: no requirement/design docs found for module '{module}'",
            file=sys.stderr,
        )
        return 1

    commit = _merge_base_commit()
    if commit is None:
        print(
            "Error: git merge-base HEAD origin/master failed. "
            "Ensure origin/master exists.",
            file=sys.stderr,
        )
        return 1

    commit_time = _commit_committer_date(commit)
    confirmed_time = _now_iso()

    records = _load_records()
    _upsert_record(
        records, module,
        commit=commit,
        commit_time=commit_time,
        confirmed_time=confirmed_time,
        comment="",
        blocked_reason="",
    )

    _save_records(records)
    print(f"Recorded test-case discovery for module '{module}' ({len(docs)} doc(s))")
    return 0


def cmd_comment(args: SimpleNamespace) -> int:
    """Override the comment for a module.

    If the module already has a record, only the comment is overwritten.
    If no record exists yet, a new record is created with an empty commit.
    """
    records = _load_records()
    is_new = not any(r["module"] == args.module for r in records)
    _upsert_record(
        records, args.module,
        comment=args.text,
        confirmed_time=_now_iso(),
    )
    _save_records(records)
    if is_new:
        print(f"Created record for module '{args.module}'")
    else:
        print(f"Updated comment for module '{args.module}'")
    return 0


def cmd_blocked(args: SimpleNamespace) -> int:
    """Mark a module as blocked with a reason.

    If the module already has a record, the blocked_reason is updated.
    If no record exists yet, a new record is created.
    """
    records = _load_records()
    is_new = not any(r["module"] == args.module for r in records)
    _upsert_record(
        records, args.module,
        blocked_reason=args.reason,
        confirmed_time=_now_iso(),
    )
    _save_records(records)
    if is_new:
        print(f"Created blocked record for module '{args.module}'")
    else:
        print(f"Updated blocked reason for module '{args.module}'")
    return 0


def cmd_check(args: SimpleNamespace) -> int:
    records = _load_records()
    record_map: Dict[str, Dict[str, str]] = {r["module"]: r for r in records}

    changed: List[str] = []

    for module in _discover_modules():
        docs = _module_docs(module)
        if not docs:
            continue
        rec = record_map.get(module)
        if rec is None:
            # no record → untracked
            changed.append(module)
            continue
        if rec["commit"] == "":
            # empty commit → treat as changed
            blocked = rec.get("blocked_reason", "")
            if blocked:
                # blocked module with empty commit → auto-unblock
                _upsert_record(records, module, blocked_reason="")
                _save_records(records)
                record_map = {r["module"]: r for r in records}
            changed.append(module)
            continue
        if _module_changed(rec["commit"], docs):
            # requirement/design docs changed since last confirmation
            blocked = rec.get("blocked_reason", "")
            if blocked:
                # blocked module updated → auto-unblock
                new_commit = _merge_base_commit() or rec["commit"]
                _upsert_record(
                    records, module,
                    blocked_reason="",
                    commit=new_commit,
                    commit_time=_commit_committer_date(new_commit),
                    confirmed_time=_now_iso(),
                )
                _save_records(records)
                record_map = {r["module"]: r for r in records}
            changed.append(module)
        else:
            # no change — skip blocked modules entirely
            blocked = rec.get("blocked_reason", "")
            if blocked:
                continue

    for module in changed:
        rec = record_map.get(module, {})
        comment = rec.get("comment", "")
        if comment:
            print(f"{module}\t{comment}")
        else:
            print(module)

    return 0


# ── main ─────────────────────────────────────────────────────────────────


@click.group()
def main() -> int:
    """Requirement E2E Tracker – 跟踪各模块测试用例的发现状态。"""
    return 0


@main.command(name="finished")
@click.argument("module")
def finished_cmd(module: str) -> int:
    """标记模块的测试用例已发现。MODULE 为模块名（对应 docs/requirements/<module>.md）。"""
    return cmd_finished(SimpleNamespace(module=module))


@main.command(name="comment")
@click.argument("module")
@click.argument("text")
def comment_cmd(module: str, text: str) -> int:
    """为模块设置/覆盖评论。MODULE 为模块名，TEXT 为评论内容。"""
    return cmd_comment(SimpleNamespace(module=module, text=text))


@main.command(name="blocked")
@click.argument("module")
@click.argument("reason")
def blocked_cmd(module: str, reason: str) -> int:
    """标记模块被阻塞。MODULE 为模块名，REASON 为阻塞原因。"""
    return cmd_blocked(SimpleNamespace(module=module, reason=reason))


@main.command(name="check")
def check_cmd() -> int:
    """扫描所有模块，报告需求/设计文档变更或未跟踪的模块。"""
    return cmd_check(SimpleNamespace())


if __name__ == "__main__":
    sys.exit(main())
