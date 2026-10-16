#!/usr/bin/env python3
"""rust-version 声明完整性扫描：workspace 成员须继承 workspace MSRV（issue #3532）。

成员 crate 的 [package] 段缺 `rust-version` 声明时，其有效 MSRV 静默回退到
构建工具链版本（clippy 的 msrv 感知 lint 以该值为准），兼容性承诺失真。
本模块只断言「声明形态为 workspace 继承」，不校验版本数值——MSRV 数值的
唯一事实来源是根 Cargo.toml 的 [workspace.package] rust-version。
"""

from __future__ import annotations

import tomllib
from pathlib import Path

_GLOB_CHARS = ("*", "?", "[")


class RustVersionError(Exception):
    """根 workspace 解析或成员 manifest 读取失败（fail-loud，不静默跳过）。"""


def _has_glob(entry: str) -> bool:
    """条目是否含 glob 通配符（cargo 的 members / exclude 均支持通配展开）。"""
    return any(ch in entry for ch in _GLOB_CHARS)


def _normalize(entry: str) -> str:
    """成员路径规范化：分隔符统一为 `/`，剥离 `./` 前缀。"""
    return Path(entry.replace("\\", "/")).as_posix()


def _glob_expand(repo_root: Path, pattern: str) -> set[str]:
    """相对 repo_root 展开单个 glob 模式，返回规范化相对路径集合。"""
    return {path.relative_to(repo_root).as_posix() for path in repo_root.glob(pattern)}


def _load_toml(manifest: Path, *, is_root: bool) -> dict:
    """tomllib 读取 manifest；根/成员 manifest 缺失或解析失败 → RustVersionError。"""
    missing_label = "根 Cargo.toml 缺失" if is_root else "成员 Cargo.toml 缺失"
    try:
        with manifest.open("rb") as fh:
            return tomllib.load(fh)
    except FileNotFoundError as exc:
        raise RustVersionError(f"{missing_label}: {manifest}") from exc
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise RustVersionError(f"解析 {manifest} 失败: {exc}") from exc


def workspace_members(repo_root: Path) -> list[tuple[str, Path]]:
    """解析根 Cargo.toml [workspace]，返回成员 (相对路径, 绝对路径) 清单。

    glob 成员条目按 cargo 语义展开（仅保留含 Cargo.toml 的目录）；展开结果
    剔除 [workspace.exclude]（同样支持 glob）；按规范化相对路径排序去重。
    无 [workspace] / members 段视为空清单（无成员可查）。
    """
    data = _load_toml(repo_root / "Cargo.toml", is_root=True)
    workspace = data.get("workspace")
    entries = workspace.get("members") if isinstance(workspace, dict) else None
    if not isinstance(entries, list):
        return []

    members: set[str] = set()
    for raw in entries:
        if not isinstance(raw, str):
            raise RustVersionError(f"members 含非字符串条目: {raw!r}")
        entry = _normalize(raw)
        if not _has_glob(entry):
            members.add(entry)
            continue
        expanded = {
            rel
            for rel in _glob_expand(repo_root, entry)
            if (repo_root / rel).is_dir() and (repo_root / rel / "Cargo.toml").is_file()
        }
        members |= expanded

    excludes: set[str] = set()
    raw_excludes = workspace.get("exclude") or []
    if isinstance(raw_excludes, list):
        for raw in raw_excludes:
            if not isinstance(raw, str):
                continue
            entry = _normalize(raw)
            excludes |= _glob_expand(repo_root, entry) if _has_glob(entry) else {entry}

    return sorted((rel, repo_root / rel) for rel in members - excludes)


def find_violations(repo_root: Path) -> list[tuple[str, str]]:
    """逐成员断言 rust-version 为 workspace 继承，返回排序后 (路径, 原因) 清单。

    判定口径：缺失 → 「未声明」；str / dict 非 `{"workspace": True}` 形态 →
    「硬编码」（不接受硬编码版本号，保证收敛到 workspace 单一事实来源）；
    成员 manifest 缺失或损坏 → RustVersionError（fail-loud）。
    """
    violations: list[tuple[str, str]] = []
    for rel, member_dir in workspace_members(repo_root):
        data = _load_toml(member_dir / "Cargo.toml", is_root=False)
        package = data.get("package")
        declared = package.get("rust-version") if isinstance(package, dict) else None
        reason = _violation_reason(declared)
        if reason is not None:
            violations.append((rel, reason))
    return sorted(violations)


def _violation_reason(declared: object) -> str | None:
    """rust-version 声明形态 → 违规原因；合规（workspace 继承）返回 None。"""
    if declared is None:
        return "rust-version 未声明"
    if declared == {"workspace": True}:
        return None
    if isinstance(declared, str):
        return f'rust-version 硬编码 "{declared}"（须 rust-version.workspace = true）'
    return f"rust-version 硬编码 {declared!r}（须 rust-version.workspace = true）"
