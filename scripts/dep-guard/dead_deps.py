#!/usr/bin/env python3
"""死依赖检查：cargo machete 未使用依赖（覆盖外部 crate）→ diff baseline。"""

from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path

VERSION_TIMEOUT_SECONDS = 60
RUN_TIMEOUT_SECONDS = 600


class DeadDepsError(Exception):
    """cargo machete 执行失败或输出不符合预期。"""


@dataclass(frozen=True)
class MacheteData:
    unused: dict[str, list[str]]
    source: str  # "json" | "text"


_PACKAGE_RE = re.compile(r"^(?P<name>\S+) -- \S*Cargo\.toml:$")
_DEP_RE = re.compile(r"^[A-Za-z0-9_.+-]+$")


def machete_available() -> bool:
    """功能性探测：cargo machete --version 成功即视为可用（缺工具 → 上层记 SKIP）。"""
    try:
        proc = subprocess.run(
            ["cargo", "machete", "--version"],
            capture_output=True,
            text=True,
            timeout=VERSION_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    return proc.returncode == 0


def _run_machete(repo_root: Path, extra_args: list[str]) -> subprocess.CompletedProcess:
    try:
        return subprocess.run(
            ["cargo", "machete", *extra_args],
            cwd=str(repo_root),
            capture_output=True,
            text=True,
            timeout=RUN_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise DeadDepsError(f"cargo machete 执行失败: {exc}") from exc


def collect_unused(repo_root: Path) -> MacheteData:
    """优先 cargo machete --json 解析；不可用（不支持/非 JSON）则回退文本输出。"""
    proc = _run_machete(repo_root, ["--json"])
    if proc.returncode in (0, 1):
        try:
            data = json.loads(proc.stdout)
        except json.JSONDecodeError:
            data = None
        if isinstance(data, list):
            return MacheteData(unused=parse_json_output(data), source="json")

    proc = _run_machete(repo_root, [])
    if proc.returncode not in (0, 1):
        stderr = (proc.stderr or proc.stdout or "").strip()[:500]
        raise DeadDepsError(f"cargo machete 失败（exit {proc.returncode}）: {stderr}")
    return MacheteData(unused=parse_text_output(proc.stdout), source="text")


def parse_json_output(data: list) -> dict[str, list[str]]:
    """解析 --json 输出：条目含 package_name / unused_deps 字段。"""
    unused: dict[str, list[str]] = {}
    for entry in data:
        if not isinstance(entry, dict) or "package_name" not in entry:
            raise DeadDepsError("machete JSON 条目缺少 package_name 字段")
        name = str(entry["package_name"])
        deps = {str(dep) for dep in entry.get("unused_deps") or []}
        merged = set(unused.get(name, [])) | deps
        if merged:
            unused[name] = sorted(merged)
    return unused


def parse_text_output(text: str) -> dict[str, list[str]]:
    """解析默认文本输出：`<pkg> -- <…Cargo.toml>:` 段后跟缩进依赖名行。

    尾部说明文字（非缩进散行）不属于任何 crate 段，忽略。
    """
    unused: dict[str, list[str]] = {}
    current: str | None = None
    for raw in text.splitlines():
        line = raw.rstrip()
        match = _PACKAGE_RE.match(line)
        if match:
            current = match.group("name")
            unused.setdefault(current, [])
            continue
        if current is None:
            continue
        dep = line.strip()
        if line[0:1] in (" ", "\t") and dep and _DEP_RE.match(dep):
            unused[current].append(dep)
        elif dep:
            current = None
    return unused


def unused_items(unused: dict[str, list[str]]) -> list[str]:
    """未使用依赖 → baseline 行格式（`<crate 包名>: <依赖名>`），排序去重。"""
    items = []
    for pkg, deps in unused.items():
        for dep in deps:
            items.append(f"{pkg}: {dep}")
    return sorted(set(items))
