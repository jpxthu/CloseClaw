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
JSONL = SCRIPT_DIR / "daily_stats.jsonl"
HTML_OUT = SCRIPT_DIR / "code_stats_chart.html"

def load_data():
    records = []
    if not JSONL.exists():
        print(f"ERROR: {JSONL} not found. Run collect_code_stats.py first.", file=sys.stderr)
        sys.exit(1)
    with open(JSONL) as f:
        for line in f:
            records.append(json.loads(line))
    return records

def estimate_time():
    """Professional Rust engineer: normal FE/BE dev pace."""
    return {
        "script_dev_hours": 8.0,
        "commit_traverse_hours": 3.2,
        "coverage_runs_hours": 6.5,
        "cleanup_hours": 4.0,
        "verify_hours": 4.0,
        "total_hours": 25.8,
        "total_days_8h": 3.22,
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
    te = estimate_time()

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

    # Coverage proxy: tests / max_tests * 100
    max_t = max(tests) or 1
    cov   = [round(t / max_t * 100, 1) for t in tests]

    # Average coverage line (horizontal)
    avg_cov = round(sum(cov) / len(cov), 1)

    d  = json.dumps(dates)
    fj = json.dumps(rs_files)
    cj = json.dumps(cum)
    lj = json.dumps(loc)
    tj = json.dumps(tests)
    cv = json.dumps(cov)

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
  .time-table h3 {{ font-size: 14px; color: #f28b82; margin-bottom: 12px; }}
  table {{ width: 100%; border-collapse: collapse; font-size: 12px; }}
  th {{ color: #9aa0a6; text-align: left; padding: 4px 8px; border-bottom: 1px solid #2d3a4a; }}
  td {{ color: #c4d4e4; padding: 5px 8px; border-bottom: 1px solid #1f2b3a; }}
  tr:last-child td {{ border: none; }}
  tr.total-row td {{ color: #81c995; font-weight: bold; background: #1f3328; }}
  .note {{ text-align: center; color: #555; font-size: 11px; margin-top: 20px; }}
  .wide {{ grid-column: 1 / -1; }}
</style>
</head>
<body>
<h1>🐟 CloseClaw — Master 分支每日代码统计</h1>
<p class="subtitle">2026-03-21 → 2026-04-28 &nbsp;|&nbsp; 39 天 &nbsp;|&nbsp; 336 commits &nbsp;|&nbsp; 巡检虾 🔭</p>

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
    <div class="chart-title">⚡ 测试覆盖率估算（proxy: tests/max%）</div>
    <canvas id="covChart"></canvas>
  </div>
  <div class="time-table">
    <h3>⏱ 专业 Rust 工程师工时估算</h3>
    <table>
      <tr><th>任务</th><th>小时</th></tr>
      <tr><td>脚本开发</td><td>{te["script_dev_hours"]}h</td></tr>
      <tr><td>遍历 336 个 commit</td><td>{te["commit_traverse_hours"]}h</td></tr>
      <tr><td>每日 llvm-cov（39天）</td><td>{te["coverage_runs_hours"]}h</td></tr>
      <tr><td>数据清洗 & 图表调整</td><td>{te["cleanup_hours"]}h</td></tr>
      <tr><td>验证 & 修复问题</td><td>{te["verify_hours"]}h</td></tr>
      <tr class="total-row"><td>合计</td><td>{te["total_hours"]}h ≈ {te["total_days_8h"]} 人天</td></tr>
    </table>
  </div>
</div>

<p class="note">⚠️ 覆盖率为 proxy（tests/max）；均值 {avg_cov}% 来自此估算。真实覆盖率需每日运行 llvm-cov</p>

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
        label: '覆盖率',
        data: {cv},
        borderColor: '#ea4335',
        borderDash: [5, 5],
        tension: 0.4,
        pointRadius: 2
      }},
      {{
        label: '均值 {avg_cov}%',
        data: Array(L.length).fill({avg_cov}),
        borderColor: 'rgba(138,180,248,0.5)',
        borderDash: [2, 4],
        pointRadius: 0,
        fill: false
      }}
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
    print(f"Written: {HTML_OUT}  (avg_cov={avg_cov}%)")

if __name__ == "__main__":
    main()