#!/usr/bin/env python3
"""
Generate a self-contained HTML chart for CloseClaw daily stats.

Usage:
    python3 draw_stats.py [--verbose] [--screenshot]
    python3 draw_stats.py --help

No external Python packages required. Chart.js loaded from CDN.
Input: scripts/daily_stats.jsonl (from collect_code_stats.py)
Output:
    scripts/code_stats_chart.html
    scripts/code_stats_chart.png  (when --screenshot is given, 2800x2475, 2x DPR)

Screenshot: Chrome headless -> 2800x4950 (2x DPR * viewport 2475 CSS px)
            then auto-cropped to 2800x2475.
Requires: google-chrome (in PATH), PIL (for crop).
"""

import argparse, json, os, shutil, subprocess, sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent.resolve()
REPO = SCRIPT_DIR.parent
DATA_DIR = SCRIPT_DIR / "data"
JSONL = DATA_DIR / "daily_stats.jsonl"
COVERAGE_HISTORY = DATA_DIR / "coverage_history.jsonl"
HTML_OUT = SCRIPT_DIR / "code_stats_chart.html"

_verbose = False
_screenshot = False


def load_coverage_history():
    """Load real coverage data from coverage_history.jsonl.
    Returns list of {date, avg_coverage} or empty list."""
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
                if "avg_coverage" in rec:
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
      人天 = 净功能点 / 古法速率 * (1 + 质量债务系数)

    style 选项（对应 DeepSeek V4 估算框架）:
      prototype  原型级（质量系数 0.2）  -> Vibe等效 ~2天生成
      mvp        MVP级（质量系数 0.8）    -> Vibe等效 ~5天迭代
      prod       生产级（质量系数 1.8）   -> Vibe等效 ~更长迭代

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
        # Vibe 等效（DeepSeek V4 参考: 原型*10, MVP*20, 生产*40）
        "vibe_days_equiv":     round(total_pd / {"prototype": 10, "mvp": 20, "prod": 40}[style], 1),
    }


HELP_TEXT = f"""CloseClaw Code Statistics HTML Chart Generator

Usage:
    python3 {os.path.basename(__file__)} [--verbose] [--screenshot]
    python3 {os.path.basename(__file__)} --help

Description:
    Read daily_stats.jsonl and generate code_stats_chart.html.
    No external Python packages (matplotlib, etc.) needed.
    Chart.js loaded from jsDelivr CDN at runtime.

Input:
    {JSONL}

Output:
    {HTML_OUT}
    {SCRIPT_DIR / 'code_stats_chart.png'}  (when --screenshot given, 2800x2475)

Options:
    --verbose, -v   Print extra info while generating
    --screenshot    Capture PNG screenshot (2800x2475, 2x DPR, auto-cropped)
    --help, -h      Show this help message

Examples:
    # Generate chart only
    python3 draw_stats.py

    # Generate chart + screenshot
    python3 draw_stats.py --screenshot
"""


def parse_args():
    parser = argparse.ArgumentParser(description="Generate CloseClaw code stats chart.")
    parser.add_argument("--verbose", "-v", action="store_true", help="extra output")
    parser.add_argument("--screenshot", action="store_true",
                        help="capture PNG screenshot (2800x2475, 2x DPR, auto-cropped)")
    args = parser.parse_args()
    return args.verbose, args.screenshot


def take_screenshot(html_path):
    """Capture 2800x2475 screenshot (2x DPR, viewport 2475 CSS px, auto-cropped)."""
    try:
        from PIL import Image
    except ImportError:
        print("WARNING: PIL not available, skipping screenshot", file=sys.stderr)
        return

    # Chrome binary
    chrome = shutil.which("google-chrome") or shutil.which("google-chrome-stable")
    if not chrome:
        print("WARNING: google-chrome not found, skipping screenshot", file=sys.stderr)
        return

    out_png = html_path.with_suffix(".png")
    # Full-size: viewport=1400x2475 CSS px -> 2800x4950 at 2x DPR
    full_png = out_png.parent / (out_png.stem + "_full.png")

    cmd = [
        chrome,
        "--headless=new",
        "--disable-gpu",
        f"--screenshot={full_png}",
        "--window-size=1400,2475",
        "--force-device-scale-factor=2",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        f"file://{html_path}",
    ]
    if _verbose:
        print(f"[screenshot] Running: {' '.join(cmd)}", file=sys.stderr)

    result = subprocess.run(cmd, capture_output=True, timeout=30)
    if result.returncode != 0:
        if result.stderr:
            for line in result.stderr.decode(errors="replace").splitlines()[:3]:
                print(f"[screenshot] Chrome stderr: {line}", file=sys.stderr)
        print(f"[screenshot] WARNING: chrome exited {result.returncode}, skipping crop",
              file=sys.stderr)
        return

    # Crop: full is 2800x4950 -> 2800x2475 (top portion, remove bottom blank)
    try:
        img = Image.open(full_png)
        cropped = img.crop((0, 0, 2800, 2475))
        cropped.save(out_png)
        if _verbose:
            print(f"[screenshot] Cropped {img.size} -> {cropped.size} -> {out_png}",
                  file=sys.stderr)
        else:
            print(f"Screenshot: {out_png}  (2800x2475, 2x DPR)")
        full_png.unlink(missing_ok=True)
    except Exception as e:
        print(f"[screenshot] WARNING: crop failed ({e}), full screenshot at {full_png}",
              file=sys.stderr)


def main():
    global _verbose, _screenshot
    _verbose, _screenshot = parse_args()

    if _verbose and "--help" not in sys.argv and "-h" not in sys.argv:
        print(HELP_TEXT)
        return

    if "--help" in sys.argv or "-h" in sys.argv:
        print(HELP_TEXT)
        return

    records = load_data()

    # 三种风格工时估算
    tp  = estimate_time_vibe("prototype")
    tm  = estimate_time_vibe("mvp")
    tpr = estimate_time_vibe("prod")

    cov_history = load_coverage_history()
    has_real_cov = len(cov_history) > 0

    # Build date -> value lookups for daily stats
    daily_dates = [r["date"] for r in records]
    daily_map = {r["date"]: r for r in records}

    # Merge dates from daily_stats and coverage_history
    all_dates = sorted(set(daily_dates) | {r["date"] for r in cov_history})

    dates    = all_dates
    rs_files = [daily_map.get(d, {}).get("rs_files") for d in all_dates]
    loc      = [daily_map.get(d, {}).get("total_loc") for d in all_dates]
    tests    = [daily_map.get(d, {}).get("test_cases") for d in all_dates]

    # Forward-fill daily stats for gap days
    def forward_fill(arr):
        result = []
        last = None
        for v in arr:
            if v is not None:
                last = v
            result.append(last)
        return result

    rs_files = forward_fill(rs_files)
    loc = forward_fill(loc)
    tests = forward_fill(tests)

    # FIX: cumulative should be running max (monotonic increase, ends at total)
    cum = []
    running = 0
    for v in loc:
        running = max(running, v)
        cum.append(running)

    cov_history_count = len(cov_history)

    # Max coverage proxy from daily stats (full timeline)
    max_t = max(tests) if tests else 1
    cov_max_proxy = [round(t / max_t * 100, 1) for t in tests]

    if has_real_cov:
        # Build date -> coverage lookup from history
        hist_avg = {r["date"]: r["avg_coverage"] for r in cov_history}

        # Avg coverage: only real data points
        cov_avg_data = [hist_avg.get(d) for d in dates]

        # Stats for display
        latest_avg = cov_history[-1]["avg_coverage"]
        cov_note = f"真实覆盖率（llvm-cov）: 最新 avg={latest_avg}% | 最高覆盖率 = proxy（tests/max）"
        cov_subtitle = "⚡ 测试覆盖率（avg 实测 + max 估算）"
    else:
        cov_avg_data = None
        proxy_avg = round(sum(cov_max_proxy) / len(cov_max_proxy), 1)
        cov_note = f"⚠️ 平均覆盖率未采集。运行 collect_coverage.py 获取真实数据"
        cov_subtitle = "⚡ 测试覆盖率估算（proxy: tests/max%）"

    d  = json.dumps(dates)
    fj = json.dumps(rs_files)
    cj = json.dumps(cum)
    lj = json.dumps(loc)
    tj = json.dumps(tests)
    cv_avg = json.dumps(cov_avg_data) if cov_avg_data else "null"
    cv_max = json.dumps(cov_max_proxy)

    # Build coverage chart HTML
    if has_real_cov:
        cov_html = f'<div class="chart-title">{cov_subtitle}</div><canvas id="covChart"></canvas>'
    else:
        cov_html = f'<div class="chart-title">{cov_subtitle}</div><canvas id="covChart"></canvas>'

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
  .cov-placeholder {{ display: flex; align-items: center; justify-content: center; height: 180px; color: #5f6b7a; font-size: 14px; text-align: center; }}
  .cov-placeholder .cov-num {{ color: #8ab4f8; font-size: 32px; font-weight: 600; display: block; margin-bottom: 8px; }}
  .cov-placeholder .cov-sub {{ color: #9aa0a6; font-size: 12px; margin-top: 4px; }}
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
    {cov_html}
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
      ...(function() {{
        var avgData = {cv_avg};
        if (avgData !== null && avgData.some(v => v !== null)) {{
          var n = avgData.filter(v => v !== null).length;
          return [{{
            label: '平均覆盖率 (%)',
            data: avgData,
            borderColor: '#ea4335',
            borderWidth: n <= 5 ? 3 : 1.5,
            tension: 0.4,
            spanGaps: true,
            pointRadius: n <= 3 ? 8 : (n <= 10 ? 5 : 2),
            pointBackgroundColor: '#ea4335',
            fill: {{ target: 'origin', above: 'rgba(234,67,53,0.08)' }}
          }}];
        }}
        return [];
      }})(),
      {{
        label: '最高覆盖率 (%)',
        data: {cv_max},
        borderColor: '#34a853',
        borderDash: [5, 5],
        tension: 0.4,
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
    if has_real_cov:
        print(f"Written: {HTML_OUT}  (real coverage: avg={latest_avg}%)")
    else:
        print(f"Written: {HTML_OUT}  (proxy coverage, no real data)")

    # Screenshot: HTML -> PNG (2800x2475)
    if _screenshot:
        take_screenshot(HTML_OUT)


if __name__ == "__main__":
    main()
