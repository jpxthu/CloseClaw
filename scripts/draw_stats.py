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
    scripts/code_stats_chart.png  (when --screenshot is given, 2800x2300, 2x DPR)

Screenshot: Chrome headless -> 2800x4600 (2x DPR * viewport 2300 CSS px)
            then auto-cropped to 2800x2300.
Requires: google-chrome (in PATH), PIL (for crop).
"""

import argparse, json, os, shutil, subprocess, sys, math
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent.resolve()
REPO = SCRIPT_DIR.parent
DATA_DIR = SCRIPT_DIR / "data"
JSONL = DATA_DIR / "daily_stats.jsonl"
COVERAGE_HISTORY = DATA_DIR / "coverage_history.jsonl"
HTML_OUT = SCRIPT_DIR / "code_stats_chart.html"

_verbose = False
_screenshot = False

# Screenshot crop
CROP_W = 2800
CROP_H = 2200
VIEWPORT_W = 1400
VIEWPORT_H = 2200   # CSS px -> 2x = 2800x4400 -> crop to 2800x2200


def load_coverage_history():
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
    {SCRIPT_DIR / 'code_stats_chart.png'}  (when --screenshot given, 2800x2200)

Options:
    --verbose, -v   Print extra info while generating
    --screenshot    Capture PNG screenshot (2800x2200, 2x DPR, auto-cropped)
    --help, -h      Show this help message

Examples:
    python3 draw_stats.py
    python3 draw_stats.py --screenshot
"""


def parse_args():
    parser = argparse.ArgumentParser(description="Generate CloseClaw code stats chart.")
    parser.add_argument("--verbose", "-v", action="store_true", help="extra output")
    parser.add_argument("--screenshot", action="store_true",
                        help="capture PNG screenshot (2800x2300, 2x DPR, auto-cropped)")
    args = parser.parse_args()
    return args.verbose, args.screenshot


def take_screenshot(html_path):
    """Capture 2800x2200 screenshot (2x DPR, viewport 2200 CSS px, auto-cropped)."""
    try:
        from PIL import Image
    except ImportError:
        print("WARNING: PIL not available, skipping screenshot", file=sys.stderr)
        return

    chrome = shutil.which("google-chrome") or shutil.which("google-chrome-stable")
    if not chrome:
        print("WARNING: google-chrome not found, skipping screenshot", file=sys.stderr)
        return

    out_png = html_path.with_suffix(".png")
    full_png = out_png.parent / (out_png.stem + "_full.png")

    cmd = [
        chrome,
        "--headless=new",
        "--disable-gpu",
        f"--screenshot={full_png}",
        f"--window-size={VIEWPORT_W},{VIEWPORT_H}",
        "--force-device-scale-factor=2",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        f"file://{html_path}",
    ]
    if _verbose:
        print(f"[screenshot] Running: {' '.join(cmd)}", file=sys.stderr)

    result = subprocess.run(cmd, capture_output=True, timeout=60)
    if result.returncode != 0:
        if result.stderr:
            for line in result.stderr.decode(errors="replace").splitlines()[:3]:
                print(f"[screenshot] Chrome stderr: {line}", file=sys.stderr)
        print(f"[screenshot] WARNING: chrome exited {result.returncode}, skipping crop",
              file=sys.stderr)
        return

    try:
        img = Image.open(full_png)
        cropped = img.crop((0, 0, CROP_W, CROP_H))
        cropped.save(out_png)
        if _verbose:
            print(f"[screenshot] Cropped {img.size} -> {cropped.size} -> {out_png}",
                  file=sys.stderr)
        else:
            print(f"Screenshot: {out_png}  ({CROP_W}x{CROP_H}, 2x DPR)")
        full_png.unlink(missing_ok=True)
    except Exception as e:
        print(f"[screenshot] WARNING: crop failed ({e}), full screenshot at {full_png}",
              file=sys.stderr)


def main():
    global _verbose, _screenshot
    _verbose, _screenshot = parse_args()

    if "--help" in sys.argv or "-h" in sys.argv:
        print(HELP_TEXT)
        return

    if _verbose:
        print(HELP_TEXT)

    records = load_data()
    cov_history = load_coverage_history()
    has_real_cov = len(cov_history) > 0

    dates    = [r["date"] for r in records]
    rs_files = [r.get("rs_files") for r in records]
    loc      = [r.get("total_loc") for r in records]
    tests    = [r.get("test_cases") for r in records]

    # Forward-fill gap days
    def forward_fill(arr):
        result, last = [], None
        for v in arr:
            if v is not None: last = v
            result.append(last)
        return result

    rs_files = forward_fill(rs_files)
    loc      = forward_fill(loc)
    tests    = forward_fill(tests)

    # Cumulative: running ADD (累加总量)
    cum_add = []
    running = 0
    for v in loc:
        running += v
        cum_add.append(running)

    # Cumulative: running MAX (历史最高)
    cum_max = []
    running = 0
    for v in loc:
        running = max(running, v)
        cum_max.append(running)

    # Coverage
    cov_map = {r["date"]: r["avg_coverage"] for r in cov_history}
    cov_data = [cov_map.get(d) for d in dates]

    # Max coverage proxy from daily stats
    max_t = max(tests) if tests else 1
    cov_max_proxy = [round(t / max_t * 100, 1) for t in tests]

    # Unique commit snapshots
    snapshot_count = len(set(r["commit"] for r in records))

    # Latest stats
    latest_avg = cov_history[-1]["avg_coverage"] if has_real_cov else None
    latest_cov_max = max(cov_max_proxy) if cov_max_proxy else 0

    d  = json.dumps(dates)
    fj = json.dumps(rs_files)
    ca = json.dumps(cum_add)
    cm = json.dumps(cum_max)
    lj = json.dumps(loc)
    tj = json.dumps(tests)
    cv_data_json = json.dumps(cov_data) if has_real_cov else "null"
    cv_max_json  = json.dumps(cov_max_proxy)

    pt_radius = min(3, max(2, len(dates) // 15))

    # Scale params for dual-axis LOC chart
    max_add = max(cum_add) if cum_add else 1
    max_max_val = max(cum_max) if cum_max else 1

    # ─── Build HTML ────────────────────────────────────────────────────────────

    if has_real_cov:
        cov_chart_section = f"""
  <div class="chart-box wide">
    <div class="chart-title">④ 测试覆盖率 &nbsp;|&nbsp; avg={latest_avg}% &nbsp; max_proxy={latest_cov_max:.1f}%</div>
    <canvas id="covChart"></canvas>
  </div>"""
        cov_js = f"""
new Chart(document.getElementById('covChart'), {{
  type: 'line',
  data: {{
    labels: L,
    datasets: [
      {{
        label: '平均覆盖率 (%)',
        data: {cv_data_json},
        borderColor: '#ea4335',
        borderWidth: 2,
        tension: 0.4,
        spanGaps: true,
        pointRadius: {pt_radius},
        pointBackgroundColor: '#ea4335',
        fill: {{ target: 'origin', above: 'rgba(234,67,53,0.08)' }}
      }},
      {{
        label: '最高覆盖率 proxy (%)',
        data: {cv_max_json},
        borderColor: '#34a853',
        borderDash: [5, 5],
        borderWidth: 1.5,
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
}});"""
    else:
        cov_chart_section = ""
        cov_js = ""

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
  h1 {{ text-align: center; color: #8ab4f8; font-size: 20px; margin-bottom: 4px; font-weight: 500; }}
  .subtitle {{ text-align: center; color: #9aa0a6; font-size: 12px; margin-bottom: 20px; }}
  .charts {{ display: flex; flex-direction: column; gap: 14px; }}
  .chart-box {{ background: #1a2332; border-radius: 10px; padding: 14px 16px; }}
  .wide {{ width: 100%; }}
  .chart-title {{ font-size: 12px; color: #b4c4d4; margin-bottom: 8px; font-weight: 500; }}
  canvas {{ max-height: 180px; }}
  .note {{ text-align: center; color: #555; font-size: 11px; margin-top: 16px; }}
</style>
</head>
<body>
<h1>🐟 CloseClaw — Master 分支每日代码统计</h1>
<p class="subtitle">{dates[0]} → {dates[-1]} &nbsp;|&nbsp; {len(dates)} 天 &nbsp;|&nbsp; {loc[-1]:,} 行 &nbsp;|&nbsp; 巡检虾 🔭</p>

<div class="charts">
  <div class="chart-box wide">
    <div class="chart-title">① 累计 Rust 代码行数（双 Y 轴：蓝=累加总量 / 绿=历史最高）</div>
    <canvas id="cumChart"></canvas>
  </div>
  <div class="chart-box wide">
    <div class="chart-title">② 源文件数 &nbsp;|&nbsp; 最高 {max(rs_files)} 个</div>
    <canvas id="filesChart"></canvas>
  </div>
  <div class="chart-box wide">
    <div class="chart-title">③ 测试用例数 &nbsp;|&nbsp; 最新 {tests[-1]} 个</div>
    <canvas id="testsChart"></canvas>
  </div>{cov_chart_section}
</div>

<p class="note">覆盖率数据来源: llvm-cov &nbsp;|&nbsp; 巡检虾 🔭</p>

<script>
const L = {d};
const baseCfg = {{
  responsive: true, animation: {{ duration: 0 }},
  plugins: {{
    legend: {{ display: false }},
    tooltip: {{ mode: 'index', intersect: false, backgroundColor: '#1a2332', titleColor: '#8ab4f8', bodyColor: '#c4d4e4' }}
  }},
  scales: {{
    x: {{ ticks: {{ color: '#5f6b7a', font: {{ size: 9 }}, maxTicksLimit: 24 }}, grid: {{ color: '#1f2b3a' }} }},
    y: {{ ticks: {{ color: '#5f6b7a', font: {{ size: 9 }} }}, grid: {{ color: '#1f2b3a' }} }}
  }}
}};

// ── ① 累计 LOC: dual Y-axis ─────────────────────────────────────────────────
// Left axis: 累加总量 (running ADD)
// Right axis: 历史最高 (running MAX)
const cumMaxVal = {max_max_val};
const cumAddMax = {max_add};
new Chart(document.getElementById('cumChart'), {{
  type: 'line',
  data: {{
    labels: L,
    datasets: [
      {{
        label: '累加总量',
        data: {ca},
        borderColor: '#4285f4',
        backgroundColor: 'rgba(66,133,244,0.1)',
        fill: true,
        tension: 0.4,
        pointRadius: 2,
        yAxisID: 'yAdd'
      }},
      {{
        label: '历史最高',
        data: {cm},
        borderColor: '#81c995',
        borderDash: [5, 5],
        tension: 0.4,
        pointRadius: 0,
        fill: false,
        yAxisID: 'yMax'
      }}
    ]
  }},
  options: {{
    ...baseCfg,
    scales: {{
      x: baseCfg.scales.x,
      yAdd: {{
        type: 'linear',
        position: 'left',
        min: 0,
        max: cumAddMax,
        ticks: {{ color: '#4285f4', font: {{ size: 9 }} }},
        grid: {{ color: '#1f2b3a' }},
        title: {{ display: true, text: '累加总量', color: '#4285f4', font: {{ size: 9 }} }}
      }},
      yMax: {{
        type: 'linear',
        position: 'right',
        min: 0,
        max: cumMaxVal,
        ticks: {{ color: '#81c995', font: {{ size: 9 }} }},
        grid: {{ drawOnChartArea: false }}
      }}
    }},
    plugins: {{ ...baseCfg.plugins, legend: {{ display: true, labels: {{ color: '#9aa0a6', font: {{ size: 10 }} }} }} }}
  }}
}});

// ── ② 源文件数 ───────────────────────────────────────────────────────────────
new Chart(document.getElementById('filesChart'), {{
  type: 'bar',
  data: {{ labels: L, datasets: [{{ data: {fj}, backgroundColor: 'rgba(251,146,60,0.7)', borderRadius: 4 }}] }},
  options: baseCfg
}});

// ── ③ 测试用例数 ──────────────────────────────────────────────────────────────
new Chart(document.getElementById('testsChart'), {{
  type: 'line',
  data: {{ labels: L, datasets: [{{ data: {tj}, borderColor: '#34a853', backgroundColor: 'rgba(52,168,83,0.1)', fill: true, tension: 0.4, pointRadius: 2 }}] }},
  options: baseCfg
}});{cov_js}
</script>
</body>
</html>"""

    with open(HTML_OUT, "w") as f:
        f.write(html)

    note = f"avg={latest_avg}%" if has_real_cov else "proxy (no real data)"
    print(f"Written: {HTML_OUT}  (coverage: {note})")

    if _screenshot:
        take_screenshot(HTML_OUT)


if __name__ == "__main__":
    main()