#!/usr/bin/env python3
"""
Generate a self-contained HTML chart for CloseClaw daily stats.

Usage:
    python3 draw_stats.py [--verbose]
    python3 draw_stats.py --help

No external Python packages required. Chart.js loaded from CDN.
Input: scripts/daily_stats.jsonl (from collect_code_stats.py)
Output: scripts/code_stats_chart.html
"""

import json, os, sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent.resolve()
REPO = SCRIPT_DIR.parent
DATA_DIR = SCRIPT_DIR / "data"
JSONL = DATA_DIR / "daily_stats.jsonl"
COVERAGE_HISTORY = DATA_DIR / "coverage_history.jsonl"
HTML_OUT = SCRIPT_DIR / "code_stats_chart.html"

def load_coverage_history():
    """Load real coverage data from coverage_history.jsonl.
    Returns list of {date, avg_coverage, max_coverage} or empty list."""
    records = []
    if not COVERAGE_HISTORY.exists():
        return records
    with open(COVERAGE_HISTORY) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
                if "avg_coverage" in rec and "max_coverage" in rec:
                    records.append(rec)
            except json.JSONDecodeError:
                pass
    return records


def load_data():
    records = []
    if not JSONL.exists():
        print(f"ERROR: {JSONL} not found. Run collect_code_stats.py first.", file=sys.stderr)
        sys.exit(1)
    with open(JSONL) as f:
        for line in f:
            records.append(json.loads(line))
    return records

def estimate_time_vibe(style="mvp"):
    """
    古法编程工时估算 — 估算对象: CloseClaw 整体工程

    基于实际产出（132 FP）套 Vibe Project 估算框架：
      人天 = 净功能点 / 古法速率 × (1 + 质量债务系数)

    style 选项（对应 DeepSeek V4 估算框架）:
      prototype  原型级（质量系数 0.2）  → Vibe等效 ~2天生成
      mvp        MVP级（质量系数 0.8）    → Vibe等效 ~5天迭代
      prod       生产级（质量系数 1.8）   → Vibe等效 ~更长迭代

    基准:
      CloseClaw 最新快照（2026-04-28）：
        170 .rs 文件 | 31830 LOC | 724 测试用例
        功能点约 132 FP（含 API/WebSocket/后台任务/前端/数据层）
    """
    fp_total = 132   # 功能点合计
    fp_rate  = 5     # FP/人天（中等复杂度 Agent 系统）

    debt_factor = {"prototype": 0.2, "mvp": 0.8, "prod": 1.8}[style]
    total_pd = fp_total / fp_rate * (1 + debt_factor)

    # 细分参考（按比例分配）
    # 脚本开发（含架构/设计）占 ~40%
    # 编码实现占 ~35%
    # 测试+调试占 ~15%
    # 文档+部署+修复占 ~10%
    total_h = total_pd * 8
    return {
        "fp_total": fp_total,
        "fp_rate": fp_rate,
        "style": style,
        "debt_factor": debt_factor,
        "script_dev_hours":   round(total_h * 0.40),
        "coding_hours":        round(total_h * 0.35),
        "testing_hours":       round(total_h * 0.15),
        "finalize_hours":      round(total_h * 0.10),
        "total_hours":         round(total_h),
        "total_pd":            round(total_pd, 1),
        # Vibe 等效（DeepSeek V4 参考: 原型×10, MVP×20, 生产×40）
        "vibe_days_equiv":     round(total_pd / {"prototype": 10, "mvp": 20, "prod": 40}[style], 1),
    }

def parse_args():
    help_flag = "--help" in sys.argv or "-h" in sys.argv
    verbose = "--verbose" in sys.argv or "-v" in sys.argv
    return help_flag, verbose

HELP_TEXT = f"""CloseClaw Code Statistics HTML Chart Generator

Usage:
    python3 {os.path.basename(__file__)} [--verbose]
    python3 {os.path.basename(__file__)} --help

Description:
    Read daily_stats.jsonl and generate code_stats_chart.html.
    No external Python packages (matplotlib, etc.) needed.
    Chart.js loaded from jsDelivr CDN at runtime.

Input:
    {JSONL}

Output:
    {HTML_OUT}

Options:
    --verbose, -v   Print extra info while generating
    --help, -h       Show this help message

Examples:
    # Generate chart
    python3 draw_stats.py

    # With verbose output
    python3 draw_stats.py --verbose
"""

_verbose = False

def main():
    global _verbose
    help_flag, _verbose = parse_args()
    if help_flag:
        print(HELP_TEXT)
        return

    records = load_data()

    # 三种风格工时估算
    tp  = estimate_time_vibe("prototype")
    tm  = estimate_time_vibe("mvp")
    tpr = estimate_time_vibe("prod")

    dates    = [r["date"] for r in records]
    rs_files = [r["rs_files"] for r in records]
    loc      = [r["total_loc"] for r in records]
    tests    = [r["test_cases"] for r in records]

    # FIX: cumulative should be running max (monotonic increase, ends at total)
    cum = []
    running = 0
    for v in loc:
        running = max(running, v)
        cum.append(running)

    cov_history = load_coverage_history()
    has_real_cov = len(cov_history) > 0

    if has_real_cov:
        # Build date → coverage lookup from history
        hist_avg = {r["date"]: r["avg_coverage"] for r in cov_history}
        hist_max = {r["date"]: r["max_coverage"] for r in cov_history}

        # Map to daily_stats dates; use None for missing days
        real_avg = [hist_avg.get(d) for d in dates]
        real_max = [hist_max.get(d) for d in dates]

        # For chart: forward-fill last known value for gap days
        def forward_fill(arr):
            result = []
            last = None
            for v in arr:
                if v is not None:
                    last = v
                result.append(last)
            return result

        cov_avg_data = forward_fill(real_avg)
        cov_max_data = forward_fill(real_max)

        # Stats for display
        latest_avg = cov_history[-1]["avg_coverage"]
        latest_max = cov_history[-1]["max_coverage"]
        cov_note = f"真实覆盖率（llvm-cov）: 最新 avg={latest_avg}%, max={latest_max}%"
        cov_subtitle = "⚡ 测试覆盖率（llvm-cov 实测）"
    else:
        # Fallback: proxy estimate
        max_t = max(tests) or 1
        cov_avg_data = [round(t / max_t * 100, 1) for t in tests]
        cov_max_data = None
        proxy_avg = round(sum(cov_avg_data) / len(cov_avg_data), 1)
        cov_note = f"⚠️ 覆盖率为 proxy（tests/max）；均值 {proxy_avg}%。运行 collect_coverage.py 获取真实数据"
        cov_subtitle = "⚡ 测试覆盖率估算（proxy: tests/max%）"

    d  = json.dumps(dates)
    fj = json.dumps(rs_files)
    cj = json.dumps(cum)
    lj = json.dumps(loc)
    tj = json.dumps(tests)
    cv_avg = json.dumps(cov_avg_data)
    cv_max = json.dumps(cov_max_data) if cov_max_data else "null"

    html = f"""<!DOCTYPE html>
<html lang="zh">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>CloseClaw Daily Code Stats</title>
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.0/dist/chart.umd.min.js"></script>
<style>
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ background: #0f1923; color: #e8eaed; font-family: 'Helvetica Neue', Arial, sans-serif; padding: 24px; }}
  h1 {{ text-align: center; color: #8ab4f8; font-size: 20px; margin-bottom: 6px; font-weight: 500; }}
  .subtitle {{ text-align: center; color: #9aa0a6; font-size: 12px; margin-bottom: 24px; }}
  .charts {{ display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }}
  .chart-box {{ background: #1a2332; border-radius: 12px; padding: 16px; }}
  .chart-title {{ font-size: 13px; color: #b4c4d4; margin-bottom: 10px; font-weight: 500; }}
  canvas {{ max-height: 200px; }}
  .bottom-grid {{ display: grid; grid-template-columns: 2fr 1fr; gap: 16px; margin-top: 16px; }}
  .time-table {{ background: #1a2332; border-radius: 12px; padding: 16px; }}
  .time-table h3 {{ font-size: 14px; color: #f28b82; margin-bottom: 6px; }}
  .time-table .est-subtitle {{ font-size: 11px; color: #789; margin-bottom: 10px; }}
  table {{ width: 100%; border-collapse: collapse; font-size: 12px; }}
  th {{ color: #9aa0a6; text-align: left; padding: 4px 8px; border-bottom: 1px solid #2d3a4a; }}
  td {{ color: #c4d4e4; padding: 5px 8px; border-bottom: 1px solid #1f2b3a; text-align: center; }}
  td:first-child {{ text-align: left; }}
  tr:last-child td {{ border: none; }}
  tr.total-row td {{ color: #81c995; font-weight: bold; background: #1f3328; }}
  tr.ratio-row td {{ color: #9aa0a6; font-size: 11px; background: #161f2a; }}
  .est-note {{ font-size: 10px; color: #567; margin-top: 8px; text-align: center; }}
  .note {{ text-align: center; color: #555; font-size: 11px; margin-top: 20px; }}
  .wide {{ grid-column: 1 / -1; }}
</style>
</head>
<body>
<h1>🐟 CloseClaw — Master 分支每日代码统计</h1>
<p class="subtitle">{dates[0]} → {dates[-1]} &nbsp;|&nbsp; {len(dates)} 天 &nbsp;|&nbsp; {loc[-1]:,} 行 &nbsp;|&nbsp; 巡检虾 🔭</p>

<div class="charts">
  <div class="chart-box wide">
    <div class="chart-title">📈 累计 Rust 代码行数（真实总量，非累加）</div>
    <canvas id="cumChart"></canvas>
  </div>
  <div class="chart-box">
    <div class="chart-title">📁 Rust 源文件数</div>
    <canvas id="filesChart"></canvas>
  </div>
  <div class="chart-box">
    <div class="chart-title">🧪 测试用例数</div>
    <canvas id="testsChart"></canvas>
  </div>
  <div class="chart-box wide">
    <div class="chart-title">📊 Rust 代码总行数（每个 commit 快照）</div>
    <canvas id="locChart"></canvas>
  </div>
</div>

<div class="bottom-grid">
  <div class="chart-box">
    <div class="chart-title">{cov_subtitle}</div>
    <canvas id="covChart"></canvas>
  </div>
  <div class="time-table">
    <h3>⏱ 古法工时估算（Vibe Project 框架）</h3>
    <p class="est-subtitle">功能点法 · {tm['fp_total']} FP · {tm['fp_rate']} FP/人天</p>
    <table>
      <tr><th>任务</th><th>原型级</th><th>MVP级</th><th>生产级</th></tr>
      <tr><td>架构 & 设计</td><td>{tp['script_dev_hours']}h</td><td>{tm['script_dev_hours']}h</td><td>{tpr['script_dev_hours']}h</td></tr>
      <tr><td>编码实现</td><td>{tp['coding_hours']}h</td><td>{tm['coding_hours']}h</td><td>{tpr['coding_hours']}h</td></tr>
      <tr><td>测试 & 调试</td><td>{tp['testing_hours']}h</td><td>{tm['testing_hours']}h</td><td>{tpr['testing_hours']}h</td></tr>
      <tr><td>文档 & 部署 & 修复</td><td>{tp['finalize_hours']}h</td><td>{tm['finalize_hours']}h</td><td>{tpr['finalize_hours']}h</td></tr>
      <tr class="total-row"><td>合计</td><td>{tp['total_pd']} 人天</td><td>{tm['total_pd']} 人天</td><td>{tpr['total_pd']} 人天</td></tr>
      <tr class="ratio-row"><td>Vibe 等效</td><td>~{int(tp['total_pd']/10)} 天</td><td>~{int(tm['total_pd']/20)} 天</td><td>~{int(tpr['total_pd']/40)} 天</td></tr>
    </table>
    <p class="est-note">质量债务系数: 原型 0.2 / MVP 0.8 / 生产 1.8 &nbsp;|&nbsp; 覆盖率为 proxy（tests/max）</p>
  </div>
</div>

<p class="note">{cov_note}</p>

<script>
const L = {d};
const baseCfg = {{
  responsive: true, animation: {{ duration: 0 }},
  plugins: {{
    legend: {{ display: false }},
    tooltip: {{ mode: 'index', intersect: false, backgroundColor: '#1a2332', titleColor: '#8ab4f8', bodyColor: '#c4d4e4' }}
  }},
  scales: {{
    x: {{ ticks: {{ color: '#5f6b7a', font: {{ size: 9 }}, maxTicksLimit: 20 }}, grid: {{ color: '#1f2b3a' }} }},
    y: {{ ticks: {{ color: '#5f6b7a', font: {{ size: 9 }} }}, grid: {{ color: '#1f2b3a' }} }}
  }}
}};

new Chart(document.getElementById('cumChart'), {{
  type: 'line',
  data: {{ labels: L, datasets: [{{ data: {cj}, borderColor: '#4285f4', backgroundColor: 'rgba(66,133,244,0.1)', fill: true, tension: 0.4, pointRadius: 2 }}] }},
  options: baseCfg
}});

new Chart(document.getElementById('filesChart'), {{
  type: 'bar',
  data: {{ labels: L, datasets: [{{ data: {fj}, backgroundColor: 'rgba(251,146,60,0.7)', borderRadius: 4 }}] }},
  options: baseCfg
}});

new Chart(document.getElementById('testsChart'), {{
  type: 'line',
  data: {{ labels: L, datasets: [{{ data: {tj}, borderColor: '#34a853', backgroundColor: 'rgba(52,168,83,0.1)', fill: true, tension: 0.4, pointRadius: 3 }}] }},
  options: baseCfg
}});

new Chart(document.getElementById('locChart'), {{
  type: 'bar',
  data: {{ labels: L, datasets: [{{ data: {lj}, backgroundColor: 'rgba(163,73,116,0.6)', borderRadius: 4 }}] }},
  options: baseCfg
}});

new Chart(document.getElementById('covChart'), {{
  type: 'line',
  data: {{
    labels: L,
    datasets: [
      {{
        label: '平均覆盖率 (%)',
        data: {cv_avg},
        borderColor: '#ea4335',
        tension: 0.4,
        pointRadius: 2
      }},
      ...(function() {{
        if ({cv_max} !== null) {{
          return [{{
            label: '最高覆盖率 (%)',
            data: {cv_max},
            borderColor: '#34a853',
            borderDash: [5, 5],
            tension: 0.4,
            pointRadius: 2
          }}];
        }}
        return [];
      }})()
    ]
  }},
  options: {{
    ...baseCfg,
    plugins: {{ ...baseCfg.plugins, legend: {{ display: true, labels: {{ color: '#9aa0a6', font: {{ size: 10 }} }} }} }},
    scales: {{ ...baseCfg.scales, y: {{ ...baseCfg.scales.y, min: 0, max: 100 }} }}
  }}
}});
</script>
</body>
</html>"""

    with open(HTML_OUT, "w") as f:
        f.write(html)
    if has_real_cov:
        print(f"Written: {HTML_OUT}  (real coverage: avg={latest_avg}%, max={latest_max}%)")
    else:
        print(f"Written: {HTML_OUT}  (proxy coverage, no real data)")

if __name__ == "__main__":
    main()