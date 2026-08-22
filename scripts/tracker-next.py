#!/usr/bin/env python3
"""
Tracker-Next – unified read-only entry over the two trackers.

Pipeline for the default ``next`` command
-----------------------------------------
1. **design stage** – mirror ``ddt.py check`` (read-only).  Any changed or
   untracked design doc → ``status: "design"`` plus the doc list
   (each entry carries its ``comment``).
2. **blocked gate** – scan the design-doc ``records.json`` for any record
   with a non-empty ``blocked_reason``.  This gate is the reason this
   script exists: ``ddt check`` *silently skips* blocked docs whose files
   have NOT changed, so an empty check output does NOT mean "all
   confirmed".  Any blocked record → ``status: "blocked-waiting"`` plus
   the blocked list.
3. **requirement stage** – mirror ``ret.py check`` (read-only).
   Non-empty → ``status: "requirement"`` plus the module list;
   empty → ``status: "empty"``.

Output: a single JSON object on stdout (exit code is always 0; the
status is conveyed by the ``status`` field)::

    {
      "status": "design" | "blocked-waiting" | "requirement" | "empty",
      "design_docs": [{"path": ..., "comment": ...}],
      "blocked_docs": [{"path": ..., "reason": ...}],
      "requirement_modules": [{"module": ..., "comment": ...}]
    }

Only the lists relevant to the reported status are non-empty.

Read-only guarantee
-------------------
Unlike ``ddt check`` / ``ret check``, this script NEVER writes any
``records.json`` (no auto-unblock persistence): querying the next step
must not dirty the working tree.  The per-key decision logic mirrors
``_tracker_core.run_check`` exactly, minus the save calls.

Requirement-side ``blocked_reason`` is intentionally NOT gated on:
blocked requirement modules stay invisible to ``ret check`` until their
docs change (the tracker's intended semantics); only design-doc blocking
pauses the whole pipeline.
"""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path
from typing import Any, Callable, Dict, List

import click

SCRIPTS_DIR = Path(__file__).resolve().parent

if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

DDT_SCRIPT = SCRIPTS_DIR / "design-doc-tracker" / "ddt.py"
RET_SCRIPT = SCRIPTS_DIR / "requirement-e2e-tracker" / "ret.py"


def _load_sibling_module(name: str, py_file: Path) -> Any:
    """Load *py_file* as module *name* (ddt.py / ret.py live in subdirs)."""
    spec = importlib.util.spec_from_file_location(name, py_file)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


ddt = _load_sibling_module("tracker_next_ddt", DDT_SCRIPT)
ret = _load_sibling_module("tracker_next_ret", RET_SCRIPT)


# ── read-only mirrors of the check state machines ───────────────────────


def _changed_keys(
    key_field: str,
    discover_keys: Callable[[], List[str]],
    key_changed: Callable[[str, str], bool],
    load_records: Callable[[], List[Dict[str, str]]],
) -> List[str]:
    """Read-only mirror of ``_tracker_core.run_check``.

    Same per-key decisions (no record / empty commit / changed →
    reported; blocked-unchanged → skipped) but WITHOUT the
    auto-unblock persistence, so records.json is never written.
    """
    record_map = {r[key_field]: r for r in load_records()}
    changed: List[str] = []
    for key in discover_keys():
        rec = record_map.get(key)
        if rec is None or rec["commit"] == "":
            changed.append(key)
            continue
        if key_changed(rec["commit"], key):
            changed.append(key)
    return changed


def _design_changed() -> List[str]:
    """Keys that ``ddt.py check`` would report right now (read-only)."""
    return _changed_keys(
        key_field="path",
        discover_keys=lambda: [
            str(p) for p in ddt._collect_md_files(ddt.DESIGN_DOC_DIR)
        ],
        key_changed=lambda commit, path: ddt._core_diff_quiet_changed(
            ddt._run, commit, [path]
        ),
        load_records=ddt._load_records,
    )


def _requirement_changed() -> List[str]:
    """Keys that ``ret.py check`` would report right now (read-only)."""
    return _changed_keys(
        key_field="module",
        discover_keys=ret._discover_modules,
        key_changed=ret._module_changed,
        load_records=ret._load_records,
    )


def _blocked_design_records() -> List[Dict[str, str]]:
    """All design-doc records with a non-empty blocked_reason.

    Scans every record, including docs no longer present in
    ``docs/design/``: anything still marked blocked keeps the pipeline
    waiting until it is resolved (``finished`` or a doc update).
    """
    return [r for r in ddt._load_records() if r.get("blocked_reason", "")]


def _comment_map(
    records: List[Dict[str, str]], key_field: str
) -> Dict[str, str]:
    return {r[key_field]: r.get("comment", "") for r in records}


# ── next command ─────────────────────────────────────────────────────────


def cmd_next() -> Dict[str, Any]:
    """Compute the pipeline status; returns the JSON payload as a dict."""
    result: Dict[str, Any] = {
        "status": "empty",
        "design_docs": [],
        "blocked_docs": [],
        "requirement_modules": [],
    }

    # 1. design stage: any changed / untracked design doc wins
    changed = _design_changed()
    if changed:
        comments = _comment_map(ddt._load_records(), "path")
        result["status"] = "design"
        result["design_docs"] = [
            {"path": p, "comment": comments.get(p, "")} for p in changed
        ]
        return result

    # 2. blocked gate: check output is empty, but blocked records may
    #    have been silently skipped — scan records.json explicitly
    blocked = _blocked_design_records()
    if blocked:
        result["status"] = "blocked-waiting"
        result["blocked_docs"] = [
            {"path": r["path"], "reason": r.get("blocked_reason", "")}
            for r in blocked
        ]
        return result

    # 3. requirement stage: design fully confirmed, scan requirements
    req_changed = _requirement_changed()
    if req_changed:
        comments = _comment_map(ret._load_records(), "module")
        result["status"] = "requirement"
        result["requirement_modules"] = [
            {"module": m, "comment": comments.get(m, "")} for m in req_changed
        ]
    return result


# ── main ─────────────────────────────────────────────────────────────────


@click.group(invoke_without_command=True)
@click.pass_context
def main(ctx: click.Context) -> int:
    """Tracker-Next – 统一「下一步」入口（design gate + requirement 扫描，只读）。"""
    if ctx.invoked_subcommand is None:
        return ctx.invoke(next_cmd)
    return 0


@main.command(name="next")
def next_cmd() -> int:
    """计算下一步工作状态，输出单个 JSON 对象（始终 rc=0）。"""
    payload = cmd_next()
    print(json.dumps(payload, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
