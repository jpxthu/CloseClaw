# Code Statistics Scripts

每日统计 CloseClaw master 分支的代码行数、文件数、测试用例数，并生成可视化图表。

## 文件说明

| 文件 | 说明 |
|------|------|
| `collect_code_stats.py` | 采集脚本：从 git 历史中按天统计（支持增量缓存，见下） |
| `cache.py` | 缓存 I/O：schema + 语义校验 + 原子写，`load_cache` 失败自动回退全量 |
| `collect_weekly_add_del.py` | 周粒度 +/- 采集：单遍 numstat，生成 `data/weekly_add_del.jsonl` |
| `collect_coverage.py` | 覆盖率采集：运行 `cargo llvm-cov`，记录真实 UT 覆盖率到 `data/coverage_history.jsonl` |
| `draw_stats.py` | 画图脚本（旧版，已废弃）：Chart.js + headless Chrome |
| `draw_stats_png.py` | 画图脚本（新版）：matplotlib 输出 `code_stats_chart.png`，四子图 sharex 对齐 |
| `data/` | 数据目录（JSONL 数据文件，已 gitignore） |
| `README.md` | 本文档 |
| `.gitignore` | 忽略生成物 |

## 快速开始

```bash
# 1. 采集代码统计（git 历史，按天）
python3 scripts/code-stats/collect_code_stats.py

# 2. 采集周 +/- 频度（画图脚本 ①b 子图依赖，单次 git log pass）
python3 scripts/code-stats/collect_weekly_add_del.py

# 3. 采集真实 UT 覆盖率（当前 HEAD，~几分钟）
python3 scripts/code-stats/collect_coverage.py

# 4. 生成图表 PNG（matplotlib）
python3 scripts/code-stats/draw_stats_png.py
#   可选：--dump-jsonl data/daily_stats.jsonl   # 同时落盘 JSONL
#   可选：--jsonl data/daily_stats.jsonl       # 从 JSONL 秒级重画，免重跑 git
#   可选：--no-cache                           # 透传给采集脚本，强制全量重采

# 5. 查看
open scripts/code-stats/code_stats_chart.png
```

每次想记录一个覆盖率数据点，运行一次 `collect_coverage.py`（同一天不会重复记录）。

## 采集脚本 — collect_code_stats.py

```
用法: python3 collect_code_stats.py [--no-cache]

按天 walk master 分支的全部 commit，统计当日 **earliest commit**（anchor）所在树的全量快照：
  - 代码文件数（.rs / .py / .sh / .js；排除 .gitignore / .example / *.json / *.lock / 等）
  - 代码总行数（排除空行、单行 // 注释、/* */ 块注释）
  - 测试用例数（Rust #[test] 属性，可含命名空间前缀，如 #[tokio::test]）
  - 测试代码行数（#[cfg(test)] 块内 + tests/ 整文件 + #[path = "..."] 包含的 _tests.rs 整文件）
  - 文档文件数（.md）
  - 文档总行数（不过滤）
  - 累计代码改动行数（每日所有 commit 的 git diff --numstat，按 |added| + |removed| 累加）
  - 快照 anchor 是当日最早 commit；当日无 commit 则 forward-fill 前一日快照。

无 JSONL 落盘。脚本把结果 **两行**打到 stdout（天数 + 首末日摘要）；不写入任何文件
（除非配置缓存，见下）。

选项:
  --no-cache   忽略 data/cache.json，强制全量重采
  --help, -h   显示本帮助信息
```

### `get_data()` 输出字段

`get_data(use_cache=True)` 返回 8 个键的 dict，供 `draw_stats_png.py` 直接消费：

| 键 | 含义 | 曲线 |
|----|------|------|
| `dates` | YYYY-MM-DD 列表（严格升序） | 共享 X 轴 |
| `code_total_loc` | 每日代码总行数 | ① 左轴 |
| `code_changed_cum` | 累计代码改动行数（\|+\| + \|-\|） | ① 右轴 |
| `doc_total_loc` | 每日文档总行数 | ② |
| `code_files` | 每日代码文件数 | ③ 左轴 |
| `doc_files` | 每日文档文件数 | ③ 右轴 |
| `test_cases` | 每日测试用例数（#[test] 属性计数） | ④ |
| `test_loc` | 每日测试代码行数 | ① 虚线 |

## 增量缓存 — data/cache.json

`collect_code_stats.py` 默认把采集结果缓存到 `data/cache.json`（已 gitignore）：

- **增量更新**：命中缓存时只 walk 缓存最后日期之后的 commit，~60× 加速（3.5min → 3s）
- **commit hash 校验**：缓存记录生成时的 HEAD；加载时 `_cache_head_is_ancestor` 校验
  该 hash 仍是当前 HEAD 的祖先。rebase / force-push / GC'd 对象 → 自动回退全量重采
  并 stderr warning（无需手动 `--no-cache`）
- **当日截断**：缓存覆盖到今天时，当天从昨日 cum 起点重建，不双计
- **同日迟到 commit 防护**：当 `today > cache_last_date` 时，新 walk 重新计算
  `cache_last_date` 当日 cum；与缓存 cum 对比，不一致 → stderr warning 并回退全量
  （避免新 churn 被静默丢弃）
- **原子写**：`pid.tmp` 唯一后缀防并发冲突，`os.replace` 原子替换；写完后
  `fsync` 父目录，崩溃不产生残缺缓存
- **schema + 语义校验**：JSON 损坏、schema 版本不符、dates 非升序、days 键缺失、
  字段类型错误 → 一律回退全量
- `--no-cache` 强制全量

> 历史：PR #1324 曾引入缓存后被 #1325 以"未经要求"revert。本次为 owner 明确要求，
> 并补齐 hash 校验、同日迟到 commit 防护、原子写、语义校验（#1324 缺失的部分）。

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

## 画图脚本 — draw_stats_png.py

```
用法: python3 draw_stats_png.py [--jsonl FILE] [--no-cache] [--out FILE] [--dump-jsonl FILE]

依赖: matplotlib（pip install matplotlib）
      中文字体 Noto Sans CJK（脚本自动检测 ~/.local/share/fonts）

输入:
  - 默认实时采集（collect_code_stats.get_data()）
  - --no-cache 透传给采集脚本，强制全量重采
  - --jsonl 指定已保存 JSONL（date 或 dates 列名均可）
输出: scripts/code-stats/code_stats_chart.png（200 dpi）
```

四张子图（共享 x 轴，纵向刻度严格对齐）：
1. **代码行数** — 左轴：代码总行数 + 测试代码行数；右轴：累计改动
2. **文档行数**
3. **源文件数** — 左轴：代码文件数；右轴：文档文件数
4. **测试用例数**

> 旧版 `draw_stats.py`（Chart.js + headless Chrome 截图）已废弃保留：各 canvas
> 自适应宽度导致 x 轴刻度纵向错位，且截图链路依赖 google-chrome + PIL 裁剪，
> 在无 Chrome 的环境直接失效。

## 覆盖率说明

### 真实覆盖率（推荐）

运行 `collect_coverage.py` 采集 `cargo llvm-cov` 数据，每次运行追加一条记录。
画图时自动使用历史数据绘制 **平均覆盖率** 和 **最高覆盖率** 两条曲线。

### Proxy 估算（fallback）

未运行过 `collect_coverage.py` 时，画图使用 proxy：`tests / max_tests × 100%`
仅反映测试数量增长趋势，不代表真实代码覆盖率。

## 工时估算（脚本自身）

采集 + 画图 + 修 bug 全套流程约 **25.8 小时**（单人），其中大部分是等待 git 操作完成。