# 定期检查

`scripts/periodic-checks.sh` 是统一入口，各段独立执行、结束打印汇总表（退出码 = 失败段数量，SKIP 不算失败；`--help` 查看完整用法）。

| 入口 | 检查目的 |
|------|----------|
| `--slow` | 慢用例清单：nextest 全量，>0.1s / >1s 两档，>5s 额外标记 SLOW |
| `--flaky` | 不稳定用例：nextest `--retries 1`，汇总重试后转绿的 FLAKY 清单；存在 FLAKY 即该段 FAIL |
| `--doctest` | 文档测试：`cargo test --workspace --doc`（nextest 不支持 doctest） |
| `--coverage` | 覆盖率：`cargo llvm-cov nextest --workspace` |
| `--deps` | 依赖审计：cargo-deny check + cargo-machete + syn 双大版本白名单检查 |
| `--dep-guard` | 依赖治理守卫：依赖方向 / 禁二次出口 / common 准入一致性 / 死依赖 / rust-version 声明完整性 + 单测套件，baseline=现状、仅对新增越界报警 |
| `--msrv` | MSRV 下限检查：以根 `Cargo.toml` 的 `rust-version` 工具链执行 `check --workspace --all-targets --locked`（工具链缺失记 SKIP） |
| `--pin-audit` | CI 版本固定守卫：action 必须 pin 40 位 commit SHA、工具版本必须显式指定（禁止 latest） |
| `--audit-selftest` | test-audit 扫描器自测 |
| `--heavy` | 重型检查：miri + TSAN（默认跳过，需显式传入，不在 `--all` 内） |
| `--all` | = slow + flaky + doctest + coverage + deps + dep-guard + msrv + pin-audit + audit-selftest（不含 heavy） |

- 工具缺失的段记 SKIP 并打印安装提示；`NEXTEST_EXTRA_ARGS` 可透传 nextest 过滤参数，如 `NEXTEST_EXTRA_ARGS="-E test(test_exec_)"`
- 合码前后跑 `--all`；周期性（如每周）跑 `--heavy`；`--slow` 的 >5s SLOW 档用于防慢测试回归
