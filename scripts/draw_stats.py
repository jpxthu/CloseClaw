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

def estimate_time_vibe(style="mvp"):
    """
    古法编程工时估算 — Vibe Project 估算框架

    style 选项:
      prototype  原型级（质量系数 0.2）  → 约 Vibe天数 × 10
      mvp        MVP级（质量系数 0.8）    → 约 Vibe天数 × 20
      prod       生产级（质量系数 1.8）   → 约 Vibe天数 × 40

    方法: 从实际产出（功能点）反推，
          考虑质量债务系数 = 修复AI代码缺失的非功能质量所需额外工时比例
    """
    # 本脚本的产出规模（功能点估算）
    # - commit 遍历逻辑          → 4 FP
    # - LOC/test 统计计数        → 5 FP
    # - HTML Chart.js 可视化     → 6 FP
    # 总计约 15 FP
    fp_total = 15

    # 古法生产率（FP/人天）
    # 内部工具，中等可靠性 → 6 FP/PD
    fp_rate = 6

    # 质量债务系数（按风格）
    #   prototype: 仅跑通，阳光路径可用      → 0.2
    #   mvp:       补充错误处理+基本测试      → 0.8
    #   prod:      重构+安全+监控+完整测试    → 1.8
    debt_factor = {"prototype": 0.2, "mvp": 0.8, "prod": 1.8}[style]

    # 总人天（含质量债务）
    total_pd = fp_total / fp_rate * (1 + debt_factor)

    # 细分项（按比例拆解，仅作参考展示）
    # 脚本开发（骨架+核心逻辑）占 ~45%
    # commit 遍历（git 命令行驱动，机械）占 ~20%
    # 数据清洗/图表调整占 ~20%
    # 验证/修复问题占 ~15%
    total_h = total_pd * 8
    return {
        "fp_total": fp_total,
        "fp_rate": fp_rate,
        "style": style,
        "debt_factor": debt_factor,
        "script_dev_hours":       round(total_h * 0.45),
        "commit_traverse_hours":  round(total_h * 0.20),
        "cleanup_hours":          round(total_h * 0.20),
        "verify_hours":           round(total_h * 0.15),
        "total_hours":            round(total_h),
        "total_pd":               round(total_pd, 1),
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
    <h3>⏱ 古法工时估算（Vibe Project 框架）</h3>
    <p class="est-subtitle">功能点法 · {tm['fp_total']} FP · {tm['fp_rate']} FP/人天</p>
    <table>
      <tr><th>任务</th><th>原型级</th><th>MVP级</th><th>生产级</th></tr>
      <tr><td>脚本开发</td><td>{tp['script_dev_hours']}h</td><td>{tm['script_dev_hours']}h</td><td>{tpr['script_dev_hours']}h</td></tr>
      <tr><td>commit 遍历</td><td>{tp['commit_traverse_hours']}h</td><td>{tm['commit_traverse_hours']}h</td><td>{tpr['commit_traverse_hours']}h</td></tr>
      <tr><td>数据清洗 & 图表</td><td>{tp['cleanup_hours']}h</td><td>{tm['cleanup_hours']}h</td><td>{tpr['cleanup_hours']}h</td></tr>
      <tr><td>验证 & 修复</td><td>{tp['verify_hours']}h</td><td>{tm['verify_hours']}h</td><td>{tpr['verify_hours']}h</td></tr>
      <tr class="total-row"><td>合计</td><td>{tp['total_pd']} 人天</td><td>{tm['total_pd']} 人天</td><td>{tpr['total_pd']} 人天</td></tr>
      <tr class="ratio-row"><td>Vibe 等效</td><td>~{int(tp['total_pd']/10)} 天</td><td>~{int(tm['total_pd']/20)} 天</td><td>~{int(tpr['total_pd']/40)} 天</td></tr>
    </table>
    <p class="est-note">质量债务系数: 原型 0.2 / MVP 0.8 / 生产 1.8 &nbsp;|&nbsp; 覆盖率为 proxy（tests/max）</p>
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