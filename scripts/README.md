# Code Statistics Scripts

每日统计 CloseClaw master 分支的代码行数、文件数、测试用例数，并生成可视化图表。

## 文件说明

| 文件 | 说明 |
|------|------|
| `collect_code_stats.py` | 采集脚本：从 git 历史中按天统计，运行后生成 `data/daily_stats.jsonl` |
| `collect_coverage.py` | 覆盖率采集：运行 `cargo llvm-cov`，记录真实 UT 覆盖率到 `data/coverage_history.jsonl` |
| `draw_stats.py` | 画图脚本：读取 JSONL，运行后生成 `code_stats_chart.html` |
| `data/` | 数据目录（JSONL 数据文件，已 gitignore） |
| `README.md` | 本文档 |
| `.gitignore` | 忽略生成物 |

## 快速开始

```bash
# 1. 采集代码统计（git 历史，按天）
python3 scripts/collect_code_stats.py

# 2. 采集真实 UT 覆盖率（当前 HEAD，~几分钟）
python3 scripts/collect_coverage.py

# 3. 生成图表
python3 scripts/draw_stats.py

# 4. 用浏览器打开
open scripts/code_stats_chart.html
```

每次想记录一个覆盖率数据点，运行一次 `collect_coverage.py`（同一天不会重复记录）。

## 采集脚本 — collect_code_stats.py

```
用法: python3 collect_code_stats.py [--verbose]

从 master 分支的每日最后一个 commit 开始，统计：
  - Rust 源文件数量（.rs）
  - 代码总行数（排除空行、单行注释 //）
  - 测试用例数量（#[test] 函数）

输出: scripts/data/daily_stats.jsonl（每行一个 JSON 对象）

选项:
  --verbose, -v   输出每日的详细处理记录
  --help, -h      显示本帮助信息
```

### 输出格式

```jsonl
{"date": "2026-03-21", "commit": "42ea4d78c81a", "rs_files": 32, "total_loc": 4786, "test_cases": 50}
{"date": "2026-03-22", "commit": "9a04565a486a", "rs_files": 37, "total_loc": 5795, "test_cases": 66}
...
```

## 覆盖率采集 — collect_coverage.py

```
用法: python3 collect_coverage.py [--verbose]

运行 cargo llvm-cov 获取当前 HEAD 的真实 UT 覆盖率。
提取: 平均覆盖率（TOTAL 行）、最高单文件覆盖率。
追加一条记录到 scripts/coverage_history.jsonl。

输出: scripts/data/coverage_history.jsonl（每行一个 JSON 对象）

选项:
  --verbose, -v   输出 llvm-cov 完整输出和解析详情
  --help, -h      显示本帮助信息
```

### 输出格式

```jsonl
{"date": "2026-05-02", "commit": "b345471", "avg_coverage": 83.44}
```

### 注意事项

- 同一天不会重复记录（删除 `data/coverage_history.jsonl` 中对应行可重新采集）
- 运行耗时取决于编译缓存状态，通常 3-10 分钟
- 需要 LLVM 工具链（通过环境变量 `LLVM_CONFIG` / `LLVM_COV` / `LLVM_PROFDATA` 指定）
- 画图时自动识别：有真实数据用 llvm-cov，没有则 fallback 到 proxy

## 画图脚本 — draw_stats.py

```
用法: python3 draw_stats.py [--verbose]

依赖: 无（纯 Python 标准库，无需 pip install）

输入:
  - scripts/data/daily_stats.jsonl（代码统计）
  - scripts/data/coverage_history.jsonl（覆盖率，可选）
输出: scripts/code_stats_chart.html（Chart.js CDN，无需安装任何 Python 包）

选项:
  --verbose, -v   显示更多信息
  --help, -h      显示本帮助信息
```

生成的 HTML 包含 6 张子图：
1. **累计 Rust 代码行数** — running max（非累加），单调递增
2. **Rust 源文件数** — 每天的 .rs 文件数量
3. **测试用例数** — #[test] 函数总数
4. **Rust 代码总行数** — 每个 commit 的快照
5. **测试覆盖率** — 真实覆盖率（avg + max）或 proxy 估算

## 数据统计（截至 2026-04-28）

| 指标 | 起始 (2026-03-21) | 结束 (2026-04-28) | 增长 |
|------|-------------------|-------------------|------|
| Rust 文件数 | 32 | 170 | 5.3× |
| 代码总行数 | 4,786 | 31,830 | 6.7× |
| 测试用例数 | 50 | 724 | 14.5× |
| 总 commit 数 | — | 336 | — |
| 统计天数 | — | 39 天 | — |

## 覆盖率说明

### 真实覆盖率（推荐）

运行 `collect_coverage.py` 采集 `cargo llvm-cov` 数据，每次运行追加一条记录。
画图时自动使用历史数据绘制 **平均覆盖率** 和 **最高覆盖率** 两条曲线。

### Proxy 估算（fallback）

未运行过 `collect_coverage.py` 时，画图使用 proxy：`tests / max_tests × 100%`
仅反映测试数量增长趋势，不代表真实代码覆盖率。

## 工时估算（脚本自身）

采集 + 画图 + 修 bug 全套流程约 **25.8 小时**（单人），其中大部分是等待 git 操作完成。
