# Code Statistics Scripts

每日统计 CloseClaw master 分支的代码行数、文件数、测试用例数，并生成可视化图表。

## 文件说明

| 文件 | 说明 |
|------|------|
| `collect_code_stats.py` | 采集脚本：从 git 历史中按天统计，输出 `daily_stats.jsonl` |
| `draw_stats.py` | 画图脚本：读取 JSONL，生成 `code_stats_chart.html` |
| `daily_stats.jsonl` | 采集结果（39 天数据） |
| `code_stats_chart.html` | 生成的图表页面（用浏览器打开即可，无需服务器） |

## 快速开始

```bash
# 1. 采集数据（默认静默模式）
python3 scripts/collect_code_stats.py

# 2. 生成图表
python3 scripts/draw_stats.py

# 3. 用浏览器打开
open scripts/code_stats_chart.html
# 或
firefox scripts/code_stats_chart.html
```

## 采集脚本 — collect_code_stats.py

```
用法: python3 collect_code_stats.py [--verbose]

从 master 分支的每日最后一个 commit 开始，统计：
  - Rust 源文件数量（.rs）
  - 代码总行数（排除空行、单行注释 //）
  - 测试用例数量（#[test] 函数）

输出: scripts/daily_stats.jsonl（每行一个 JSON 对象）

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

## 画图脚本 — draw_stats.py

```
用法: python3 draw_stats.py [--verbose]

依赖: 无（纯 Python 标准库，无需 pip install）

输入: scripts/daily_stats.jsonl
输出: scripts/code_stats_chart.html（Chart.js CDN，无需安装任何 Python 包）

选项:
  --verbose, -v   显示更多信息
  --help, -h      显示本帮助信息
```

生成的 HTML 包含 5 张子图：
1. **累计 Rust 代码行数** — running max（非累加），单调递增
2. **Rust 源文件数** — 每天的 .rs 文件数量
3. **测试用例数** — #[test] 函数总数
4. **Rust 代码总行数** — 每个 commit 的快照
5. **测试覆盖率估算** — proxy (tests/max)，含均值线

## 数据统计（截至 2026-04-28）

| 指标 | 起始 (2026-03-21) | 结束 (2026-04-28) | 增长 |
|------|-------------------|-------------------|------|
| Rust 文件数 | 32 | 170 | 5.3× |
| 代码总行数 | 4,786 | 31,830 | 6.7× |
| 测试用例数 | 50 | 724 | 14.5× |
| 总 commit 数 | — | 336 | — |
| 统计天数 | — | 39 天 | — |

## 覆盖率说明

覆盖率数据为 **proxy 估算**：`tests / max_tests × 100%`
原因：真实覆盖率需每日运行 `cargo llvm-cov`，耗时较长（约 6.5 小时/39天）。

## 工时估算（脚本自身）

采集 + 画图 + 修 bug 全套流程约 **25.8 小时**（单人），其中大部分是等待 git 操作完成。

（写 35K 行代码本身的工时不包含在此估算内，按 Rust 专业工程师 100 LOC/天算约需 318 人天）