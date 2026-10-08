#!/usr/bin/env python3
"""依赖方向检查：cargo metadata → workspace 内部常规依赖边 → 对照允许边表。"""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path

import allow_table
from allow_table import ALLOW_ALL, ROOT_CRATE


class DepEdgesError(Exception):
    """cargo metadata 执行失败或输出不符合预期。"""


@dataclass(frozen=True)
class Edge:
    from_dir: str
    to_dir: str


@dataclass
class EdgeData:
    edges: list[Edge]
    dir_to_pkg: dict[str, str]
    root_dir: str


def cargo_metadata(repo_root: Path) -> dict:
    """在 repo_root 执行 cargo metadata --no-deps，返回解析后的 JSON。"""
    proc = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=str(repo_root),
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        stderr = (proc.stderr or "").strip()[:500]
        raise DepEdgesError(f"cargo metadata 失败: {stderr}")
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise DepEdgesError(f"cargo metadata 输出解析失败: {exc}") from exc


def parse_metadata(meta: dict) -> EdgeData:
    """解析 cargo metadata：workspace 成员的常规依赖边（kind 为 null）。

    包名（连字符）↔ 目录名换算经 manifest_path 落实；根 crate 为
    manifest 位于 workspace_root 的成员。
    """
    try:
        workspace_root = meta["workspace_root"]
        member_ids = list(meta["workspace_members"])
        packages = meta["packages"]
    except (KeyError, TypeError) as exc:
        raise DepEdgesError(f"cargo metadata 缺少字段: {exc}") from exc

    id_to_pkg = {pkg["id"]: pkg for pkg in packages}
    root_manifest = str(Path(workspace_root) / "Cargo.toml")
    root_dir = None
    name_to_dir: dict[str, str] = {}
    dir_to_pkg: dict[str, str] = {}
    for member_id in member_ids:
        pkg = id_to_pkg.get(member_id)
        if pkg is None:
            raise DepEdgesError(f"workspace member 不在 packages 中: {member_id}")
        member_dir = Path(pkg["manifest_path"]).parent.name
        if str(pkg["manifest_path"]) == root_manifest:
            root_dir = member_dir
        name_to_dir[pkg["name"]] = member_dir
        dir_to_pkg[member_dir] = pkg["name"]

    edges: list[Edge] = []
    for member_id in member_ids:
        pkg = id_to_pkg[member_id]
        from_dir = Path(pkg["manifest_path"]).parent.name
        for dep in pkg.get("dependencies", []):
            if dep.get("kind") is not None:
                continue
            to_dir = name_to_dir.get(dep["name"])
            if to_dir is None or to_dir == from_dir:
                continue
            edges.append(Edge(from_dir, to_dir))

    return EdgeData(edges=edges, dir_to_pkg=dir_to_pkg, root_dir=root_dir or "")


def find_violations(
    edges: list[Edge], table: dict, root_dir: str
) -> list[Edge]:
    """对照允许边表得越界边集合（表外 crate 默认领域层规则）。"""
    violations: list[Edge] = []
    for edge in edges:
        allowed = _effective_allowed(table, edge.from_dir, root_dir)
        if allowed == ALLOW_ALL:
            continue
        if edge.to_dir in allowed:
            continue
        violations.append(edge)
    return sorted(set(violations), key=lambda edge: (edge.from_dir, edge.to_dir))


def _effective_allowed(table: dict, from_dir: str, root_dir: str) -> frozenset[str] | str:
    key = ROOT_CRATE if from_dir == root_dir and root_dir else from_dir
    allowed = table.get(key)
    if allowed is None:
        return allow_table.DEFAULT_DOMAIN_ALLOWED
    return allowed


def violation_items(violations: list[Edge], dir_to_pkg: dict[str, str]) -> list[str]:
    """越界边 → baseline 行格式（crate 包名，`closeclaw-a -> closeclaw-b`）。"""
    items = []
    for edge in violations:
        src = dir_to_pkg.get(edge.from_dir, edge.from_dir)
        dst = dir_to_pkg.get(edge.to_dir, edge.to_dir)
        items.append(f"{src} -> {dst}")
    return sorted(items)
