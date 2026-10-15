#!/usr/bin/env python3
"""dep-guard 单元测试（python3 -m unittest discover -s scripts/dep-guard）。"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest import mock

import allow_table
import baseline
import dep_edges
import dep_guard
import second_export

FIXTURE_STANDARDS = """# 产品文档写作标准

### 依赖方向允许边表

**判定**：以 crate 为单位，其 workspace 内部依赖必须落在下表内。

| 层 | crate（目录名） | 允许依赖的 workspace crate |
|----|-------|--------------------------|
| 基础层 | `common`、`platform`、`debug_log` | 无 |
| 领域层 | `agent`、`config` | `common`、`platform`、`debug_log` |
| 领域层（跨域桥接） | `gateway` | `common`、`platform`、`debug_log`、`session`、`llm`、`permission`、`config` |
| 组合根 | `daemon`、根 crate `closeclaw` | 全部 workspace crate |
| 入口客户端 | `cli` | `common`、`platform`、`debug_log`、`gateway`、`config` |
| 测试基础设施 | `fake_llm` | 无（不引用任何 CloseClaw crate） |

未列入本表的 crate 默认适用领域层规则（仅基础层）。
"""


def _fake_metadata() -> dict:
    return {
        "workspace_root": "/r",
        "workspace_members": ["root", "adapter", "common", "debug"],
        "packages": [
            {
                "id": "root",
                "name": "closeclaw-e2e",
                "manifest_path": "/r/Cargo.toml",
                "dependencies": [
                    {"name": "closeclaw-im-adapter", "kind": None},
                    {"name": "closeclaw-common", "kind": "dev"},
                ],
            },
            {
                "id": "adapter",
                "name": "closeclaw-im-adapter",
                "manifest_path": "/r/crates/im_adapter/Cargo.toml",
                "dependencies": [
                    {"name": "closeclaw-common", "kind": None},
                    {"name": "closeclaw-debug-log", "kind": None},
                    {"name": "closeclaw-common", "kind": "dev"},
                    {"name": "cc-build-script", "kind": "build"},
                ],
            },
            {
                "id": "common",
                "name": "closeclaw-common",
                "manifest_path": "/r/crates/common/Cargo.toml",
                "dependencies": [],
            },
            {
                "id": "debug",
                "name": "closeclaw-debug-log",
                "manifest_path": "/r/crates/debug_log/Cargo.toml",
                "dependencies": [],
            },
        ],
    }


class AllowTableTest(unittest.TestCase):
    def test_parse_normal(self) -> None:
        table = allow_table.parse_allow_table(FIXTURE_STANDARDS)
        self.assertEqual(table["common"], frozenset())
        self.assertEqual(table["fake_llm"], frozenset())
        self.assertEqual(
            table["agent"], frozenset({"common", "platform", "debug_log"})
        )
        self.assertEqual(
            table["gateway"],
            frozenset(
                {"common", "platform", "debug_log", "session", "llm", "permission", "config"}
            ),
        )
        self.assertEqual(
            table["cli"],
            frozenset({"common", "platform", "debug_log", "gateway", "config"}),
        )
        self.assertEqual(table["daemon"], allow_table.ALLOW_ALL)
        self.assertEqual(table[allow_table.ROOT_CRATE], allow_table.ALLOW_ALL)

    def test_parse_missing_section(self) -> None:
        with self.assertRaises(allow_table.AllowTableError):
            allow_table.parse_allow_table("# 其他文档\n正文\n")

    def test_parse_table_missing(self) -> None:
        with self.assertRaises(allow_table.AllowTableError):
            allow_table.parse_allow_table("### 依赖方向允许边表\n\n正文没有表格\n")

    def test_parse_broken_row_cell_count(self) -> None:
        text = FIXTURE_STANDARDS.replace(
            "| 基础层 | `common`、`platform`、`debug_log` | 无 |",
            "| 基础层 | `common`、`platform`、`debug_log` |",
        )
        with self.assertRaises(allow_table.AllowTableError):
            allow_table.parse_allow_table(text)

    def test_parse_crate_cell_empty(self) -> None:
        text = FIXTURE_STANDARDS.replace(
            "| 基础层 | `common`、`platform`、`debug_log` | 无 |",
            "| 基础层 | 无 | 无 |",
        )
        with self.assertRaises(allow_table.AllowTableError):
            allow_table.parse_allow_table(text)

    def test_parse_allowed_cell_unparsable(self) -> None:
        text = FIXTURE_STANDARDS.replace(
            "| 基础层 | `common`、`platform`、`debug_log` | 无 |",
            "| 基础层 | `common`、`platform`、`debug_log` | 看情况而定 |",
        )
        with self.assertRaises(allow_table.AllowTableError):
            allow_table.parse_allow_table(text)

    def test_parse_zero_data_rows(self) -> None:
        text = "\n".join(
            line
            for line in FIXTURE_STANDARDS.splitlines()
            if not line.startswith("| ") or line.startswith("| 层 ")
        )
        with self.assertRaises(allow_table.AllowTableError):
            allow_table.parse_allow_table(text)

    def test_load_from_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "STANDARDS.md"
            path.write_text(FIXTURE_STANDARDS, encoding="utf-8")
            table = allow_table.load_allow_table(path)
        self.assertIn("gateway", table)


class DepEdgesTest(unittest.TestCase):
    def test_parse_metadata_kind_and_name_mapping(self) -> None:
        data = dep_edges.parse_metadata(_fake_metadata())
        self.assertEqual(data.root_dir, "r")
        self.assertEqual(
            data.dir_to_pkg,
            {
                "r": "closeclaw-e2e",
                "im_adapter": "closeclaw-im-adapter",
                "common": "closeclaw-common",
                "debug_log": "closeclaw-debug-log",
            },
        )
        edges = {(edge.from_dir, edge.to_dir) for edge in data.edges}
        self.assertEqual(
            edges,
            {
                ("r", "im_adapter"),
                ("im_adapter", "common"),
                ("im_adapter", "debug_log"),
            },
        )

    def test_parse_metadata_missing_field(self) -> None:
        with self.assertRaises(dep_edges.DepEdgesError):
            dep_edges.parse_metadata({"workspace_root": "/r"})

    def test_violations_root_all_and_default_rule(self) -> None:
        data = dep_edges.parse_metadata(_fake_metadata())
        table = {allow_table.ROOT_CRATE: allow_table.ALLOW_ALL}
        violations = dep_edges.find_violations(data.edges, table, data.root_dir)
        self.assertEqual(violations, [])

    def test_violations_detect_out_of_table_edge(self) -> None:
        data = dep_edges.parse_metadata(_fake_metadata())
        table = {
            allow_table.ROOT_CRATE: allow_table.ALLOW_ALL,
            "im_adapter": frozenset({"common"}),
        }
        violations = dep_edges.find_violations(data.edges, table, data.root_dir)
        self.assertEqual(
            [(edge.from_dir, edge.to_dir) for edge in violations],
            [("im_adapter", "debug_log")],
        )
        items = dep_edges.violation_items(violations, data.dir_to_pkg)
        self.assertEqual(items, ["closeclaw-im-adapter -> closeclaw-debug-log"])


class BaselineTest(unittest.TestCase):
    def test_load_skips_comments_and_blank(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "b.txt"
            path.write_text("# 注释\n\n  a -> b  \n# 另一条注释\na -> b\nb -> c\n", encoding="utf-8")
            items = baseline.load_baseline(path)
        self.assertEqual(items, ["a -> b", "b -> c"])

    def test_diff_new_items_fail(self) -> None:
        diff = baseline.diff_baseline(["a"], ["a", "b"])
        self.assertEqual(diff.new_items, ["b"])
        self.assertEqual(diff.eliminated_items, [])
        self.assertTrue(diff.has_new)

    def test_diff_eliminated_items_info(self) -> None:
        diff = baseline.diff_baseline(["a", "b"], ["a"])
        self.assertEqual(diff.new_items, [])
        self.assertEqual(diff.eliminated_items, ["b"])
        self.assertFalse(diff.has_new)

    def test_diff_consistent(self) -> None:
        diff = baseline.diff_baseline(["a", "b"], ["b", "a"])
        self.assertEqual(diff.new_items, [])
        self.assertEqual(diff.eliminated_items, [])


class RunChecksTest(unittest.TestCase):
    def test_unregistered_check_skips(self) -> None:
        results, fail_count = dep_guard.run_checks(Path("."), "not-a-check")
        self.assertEqual(results[0].check_id, "not-a-check")
        self.assertEqual(results[0].status, dep_guard.STATUS_SKIP)
        self.assertEqual(fail_count, 0)

    def test_all_check_ids_registered(self) -> None:
        self.assertEqual(set(dep_guard.CHECK_RUNNERS), set(dep_guard.CHECK_IDS))

    def test_only_filter_selects_one(self) -> None:
        class FakeRunner:
            def __init__(self) -> None:
                self.called = False

            def __call__(self, repo_root: Path) -> dep_guard.CheckResult:
                self.called = True
                return dep_guard.CheckResult("dep-edges", dep_guard.STATUS_PASS, [])

        runner = FakeRunner()
        original = dep_guard.CHECK_RUNNERS["dep-edges"]
        dep_guard.CHECK_RUNNERS["dep-edges"] = runner
        try:
            results, fail_count = dep_guard.run_checks(Path("."), "dep-edges")
        finally:
            dep_guard.CHECK_RUNNERS["dep-edges"] = original
        self.assertTrue(runner.called)
        self.assertEqual(results[0].status, dep_guard.STATUS_PASS)
        self.assertEqual(fail_count, 0)


class CheckErrorPathTest(unittest.TestCase):
    """错误路径：解析 / 元数据异常 → 对应项 FAIL（mock 驱动，不触达真实仓库）。"""

    def test_dep_edges_allow_table_error_fails(self) -> None:
        with mock.patch.object(
            allow_table,
            "load_allow_table",
            side_effect=allow_table.AllowTableError("允许边表解析失败"),
        ):
            result = dep_guard.check_dep_edges(Path("."))
        self.assertEqual(result.status, dep_guard.STATUS_FAIL)
        self.assertIn("允许边表解析失败", result.lines[1])

    def test_dep_edges_metadata_error_fails(self) -> None:
        with (
            mock.patch.object(allow_table, "load_allow_table", return_value={}),
            mock.patch.object(
                dep_edges,
                "cargo_metadata",
                side_effect=dep_edges.DepEdgesError("cargo metadata 失败"),
            ),
        ):
            result = dep_guard.check_dep_edges(Path("."))
        self.assertEqual(result.status, dep_guard.STATUS_FAIL)
        self.assertIn("cargo metadata 失败", result.lines[1])

    def test_second_exports_metadata_error_fails(self) -> None:
        with mock.patch.object(
            dep_edges,
            "cargo_metadata",
            side_effect=dep_edges.DepEdgesError("cargo metadata 失败"),
        ):
            result = dep_guard.check_second_exports(Path("."))
        self.assertEqual(result.status, dep_guard.STATUS_FAIL)
        self.assertIn("cargo metadata 失败", result.lines[1])


class CheckSecondExportsDiffTest(unittest.TestCase):
    """second-exports 检查 baseline diff 集成测试（去行号归一化判定语义）。

    参照 CheckDeadDepsTest：mock 扫描层（find_second_exports），baseline 经
    临时目录真实加载，走真实 diff_baseline（key=strip_lineno）与判定链路。
    """

    BASE = "closeclaw-gateway: src/lib.rs:132 closeclaw_common::Foo"
    DRIFTED = "closeclaw-gateway: src/lib.rs:136 closeclaw_common::Foo"

    def _run_check(
        self, tmp: str, baseline_content: str, actual_items: list[str]
    ) -> dep_guard.CheckResult:
        baseline_dir = Path(tmp)
        (baseline_dir / "second-exports.txt").write_text(baseline_content, encoding="utf-8")
        with (
            mock.patch.object(dep_edges, "cargo_metadata", return_value=_fake_metadata()),
            mock.patch.object(second_export, "find_second_exports", return_value=actual_items),
            mock.patch.object(dep_guard, "BASELINE_DIR", baseline_dir),
        ):
            return dep_guard.check_second_exports(Path("."))

    def test_lineno_only_drift_passes(self) -> None:
        """仅行号漂移（132→136）→ PASS；旧整行 diff 语义下同一输入会误 FAIL。"""
        with tempfile.TemporaryDirectory() as tmp:
            result = self._run_check(tmp, f"{self.BASE}\n", [self.DRIFTED])
        self.assertEqual(result.status, dep_guard.STATUS_PASS)
        self.assertTrue(any("新增 0" in line for line in result.lines))
        # 证伪旧行为：同一输入在旧语义（key=None 整行比较）下报新增 → FAIL。
        self.assertTrue(baseline.diff_baseline([self.BASE], [self.DRIFTED]).has_new)

    def test_true_new_export_fails_with_original_entry(self) -> None:
        """真实新增二次出口 → FAIL，报文含原始条目（行号保留作定位信息）。"""
        new_entry = "closeclaw-llm: src/lib.rs:2 closeclaw_common::Qux"
        with tempfile.TemporaryDirectory() as tmp:
            result = self._run_check(tmp, f"{self.BASE}\n", [self.DRIFTED, new_entry])
        self.assertEqual(result.status, dep_guard.STATUS_FAIL)
        self.assertTrue(any(f"新增二次出口: {new_entry}" in line for line in result.lines))

    def test_eliminated_export_passes_with_info(self) -> None:
        """baseline 条目被消除 → PASS 并按「消除」语义提示可收窄。"""
        gone = "closeclaw-llm: src/lib.rs:1 closeclaw_common::Bar"
        with tempfile.TemporaryDirectory() as tmp:
            result = self._run_check(tmp, f"{self.BASE}\n{gone}\n", [self.DRIFTED])
        self.assertEqual(result.status, dep_guard.STATUS_PASS)
        self.assertTrue(
            any(f"可收窄 baseline（二次出口已消除）: {gone}" in line for line in result.lines)
        )

    def test_duplicate_lineno_in_baseline_no_false_positive(self) -> None:
        """归一化键冲突：baseline 同条目两个行号并存 → 不误报新增。"""
        with tempfile.TemporaryDirectory() as tmp:
            result = self._run_check(tmp, f"{self.BASE}\n{self.DRIFTED}\n", [self.DRIFTED])
        self.assertEqual(result.status, dep_guard.STATUS_PASS)
        self.assertTrue(any("新增 0" in line for line in result.lines))


class CliExitCodeTest(unittest.TestCase):
    """CLI 退出码 = FAIL 项数（click group 须显式 ctx.exit 透传）。"""

    def _invoke_check(self, statuses: dict[str, str]) -> int:
        from click.testing import CliRunner

        def make_runner(check_id: str, status: str):
            return lambda repo_root: dep_guard.CheckResult(check_id, status, [])

        originals = dict(dep_guard.CHECK_RUNNERS)
        dep_guard.CHECK_RUNNERS.update(
            {check_id: make_runner(check_id, status) for check_id, status in statuses.items()}
        )
        try:
            result = CliRunner().invoke(dep_guard.main, ["check"])
        finally:
            dep_guard.CHECK_RUNNERS.clear()
            dep_guard.CHECK_RUNNERS.update(originals)
        return result.exit_code

    def test_exit_code_equals_fail_count(self) -> None:
        statuses = {
            "dep-edges": dep_guard.STATUS_FAIL,
            "second-exports": dep_guard.STATUS_PASS,
            "common-admission": dep_guard.STATUS_FAIL,
            "dead-deps": dep_guard.STATUS_SKIP,
        }
        self.assertEqual(self._invoke_check(statuses), 2)

    def test_exit_code_zero_when_no_fail(self) -> None:
        statuses = {check_id: dep_guard.STATUS_PASS for check_id in dep_guard.CHECK_IDS}
        self.assertEqual(self._invoke_check(statuses), 0)


if __name__ == "__main__":
    unittest.main()
