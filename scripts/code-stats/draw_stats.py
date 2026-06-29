#!/usr/bin/env python3
"""
Generate a self-contained HTML chart for CloseClaw daily stats.

Usage:
    python3 draw_stats.py [--screenshot]

No external Python packages required. Chart.js loaded from CDN.
Input: in-memory data from ``collect_code_stats.get_data()``
Output:
    scripts/code-stats/code_stats_chart.html
    scripts/code-stats/code_stats_chart.png  (when --screenshot is given, 2800x2200)

Screenshot: Chrome headless -> 2800x4400 (2x DPR * viewport 2200 CSS px)
            then auto-cropped to 2800x2200.
Requires: google-chrome (in PATH), PIL (for crop).
"""

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

# Import data collector
sys.path.insert(0, str(Path(__file__).parent.resolve()))
from collect_code_stats import get_data

SCRIPT_DIR = Path(__file__).parent.resolve()
HTML_OUT = SCRIPT_DIR / "code_stats_chart.html"

# Screenshot crop
CROP_W = 2800
CROP_H = 2200
VIEWPORT_W = 1400
VIEWPORT_H = 2200   # CSS px -> 2x = 2800x4400 -> crop to 2800x2200


# ---------- Data preprocessing -------------------------------------------------

def forward_fill(arr):
    """Replace None with the previous non-None value. Initial Nones become 0."""
    result, last = [], 0
    for v in arr:
        if v is not None:
            last = v
        result.append(last)
    return result


# ---------- Screenshot ---------------------------------------------------------

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
        print(f"Screenshot: {out_png}  ({CROP_W}x{CROP_H}, 2x DPR)")
        full_png.unlink(missing_ok=True)
    except Exception as e:
        print(f"[screenshot] WARNING: crop failed ({e}), full screenshot at {full_png}",
              file=sys.stderr)


# ---------- CLI ----------------------------------------------------------------

def parse_args():
    parser = argparse.ArgumentParser(description="Generate CloseClaw code stats chart.")
    parser.add_argument("--screenshot", action="store_true",
                        help="capture PNG screenshot (2800x2200, 2x DPR, auto-cropped)")
    return parser.parse_args()


# ---------- Main ---------------------------------------------------------------

def build_html(data):
    """Build the HTML string from the data dict."""
    dates = data["dates"]
    if not dates:
        return None

    # Safety forward-fill (collect already does this; defensive in case of None).
    code_total_loc   = forward_fill(data["code_total_loc"])
    code_changed_cum = forward_fill(data["code_changed_cum"])
    doc_total_loc    = forward_fill(data["doc_total_loc"])
    code_files       = forward_fill(data["code_files"])
    doc_files        = forward_fill(data["doc_files"])
    test_cases       = forward_fill(data["test_cases"])
    test_loc         = forward_fill(data.get("test_loc", []))

    # Pre-computed axis max (avoids Chart.js auto-scaling desync on dual axes).
    max_code_loc    = max(max(code_total_loc), max(test_loc) if test_loc else 0) if code_total_loc else 0
    max_code_cum    = max(code_changed_cum) if code_changed_cum else 0
    max_code_files  = max(code_files)       if code_files       else 0
    max_doc_files   = max(doc_files)        if doc_files        else 0

    # Latest values (shown in chart titles)
    latest_code_loc   = code_total_loc[-1]
    latest_code_cum   = code_changed_cum[-1]
    latest_doc_loc    = doc_total_loc[-1]
    latest_code_files = code_files[-1]
    latest_doc_files  = doc_files[-1]
    latest_tests      = test_cases[-1]
    latest_test_loc   = test_loc[-1] if test_loc else 0

    # Serialize to JSON for JS injection
    d  = json.dumps(dates)
    cl = json.dumps(code_total_loc)
    cc = json.dumps(code_changed_cum)
    dl = json.dumps(doc_total_loc)
    cf = json.dumps(code_files)
    df = json.dumps(doc_files)
    tc = json.dumps(test_cases)
    tl = json.dumps(test_loc)

    pt_radius = min(3, max(2, len(dates) // 15))

    n_days = len(dates)
    date0 = dates[0]
    dateN = dates[-1]

    return f"""<!DOCTYPE html>
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
<p class="subtitle">{date0} → {dateN} &nbsp;|&nbsp; {n_days} 天 &nbsp;|&nbsp; 巡检虾 🔭</p>

<div class="charts">
  <div class="chart-box wide">
    <div class="chart-title">① 代码行数 &nbsp;|&nbsp; 左轴: 代码总行数 <b style="color:#4285f4">{latest_code_loc:,}</b> 行 &nbsp;|&nbsp; 测试代码 <b style="color:#4285f4">{latest_test_loc:,}</b> 行 &nbsp;|&nbsp; 右轴: 累计改动 <b style="color:#ea4335">{latest_code_cum:,}</b> 行</div>
    <canvas id="codeLocChart"></canvas>
  </div>
  <div class="chart-box wide">
    <div class="chart-title">② 文档行数 &nbsp;|&nbsp; 最新 <b style="color:#46bdc6">{latest_doc_loc:,}</b> 行</div>
    <canvas id="docLocChart"></canvas>
  </div>
  <div class="chart-box wide">
    <div class="chart-title">③ 源文件数 &nbsp;|&nbsp; 左轴: 代码文件数 <b style="color:#fb923c">{latest_code_files}</b> 个 &nbsp;|&nbsp; 右轴: 文档文件数 <b style="color:#9aa0a6">{latest_doc_files}</b> 个</div>
    <canvas id="filesChart"></canvas>
  </div>
  <div class="chart-box wide">
    <div class="chart-title">④ 测试用例数 &nbsp;|&nbsp; 最新 <b style="color:#34a853">{latest_tests}</b> 个</div>
    <canvas id="testsChart"></canvas>
  </div>
</div>

<p class="note">巡检虾 🔭</p>

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

// ── ① 代码行数: dual Y-axis (左: 代码总行数 / 右: 累计改动) ─────────────
new Chart(document.getElementById('codeLocChart'), {{
  type: 'line',
  data: {{
    labels: L,
    datasets: [
      {{
        label: '代码总行数',
        data: {cl},
        borderColor: '#4285f4',
        backgroundColor: 'rgba(66,133,244,0.12)',
        fill: true,
        tension: 0.4,
        pointRadius: {pt_radius},
        yAxisID: 'yLeft'
      }},
      {{
        label: '累计改动',
        data: {cc},
        borderColor: '#ea4335',
        borderDash: [5, 5],
        fill: false,
        tension: 0.4,
        pointRadius: 0,
        yAxisID: 'yRight'
      }},
      {{
        label: '测试代码行数',
        data: {tl},
        borderColor: '#4285f4',
        borderDash: [6, 4],
        fill: false,
        tension: 0.4,
        pointRadius: 0,
        yAxisID: 'yLeft'
      }}
    ]
  }},
  options: {{
    ...baseCfg,
    scales: {{
      x: baseCfg.scales.x,
      yLeft: {{
        type: 'linear',
        position: 'left',
        min: 0,
        max: {max_code_loc},
        ticks: {{ color: '#4285f4', font: {{ size: 9 }} }},
        grid: {{ color: '#1f2b3a' }},
        title: {{ display: true, text: '代码总行数', color: '#4285f4', font: {{ size: 9 }} }}
      }},
      yRight: {{
        type: 'linear',
        position: 'right',
        min: 0,
        max: {max_code_cum},
        ticks: {{ color: '#ea4335', font: {{ size: 9 }} }},
        grid: {{ drawOnChartArea: false }},
        title: {{ display: true, text: '累计改动', color: '#ea4335', font: {{ size: 9 }} }}
      }}
    }},
    plugins: {{ ...baseCfg.plugins, legend: {{ display: true, labels: {{ color: '#9aa0a6', font: {{ size: 10 }} }} }} }}
  }}
}});

// ── ② 文档行数 ────────────────────────────────────────────────────────────
new Chart(document.getElementById('docLocChart'), {{
  type: 'line',
  data: {{
    labels: L,
    datasets: [{{
      label: '文档总行数',
      data: {dl},
      borderColor: '#46bdc6',
      backgroundColor: 'rgba(70,189,198,0.12)',
      fill: true,
      tension: 0.4,
      pointRadius: {pt_radius}
    }}]
  }},
  options: baseCfg
}});

// ── ③ 源文件数: dual Y-axis (左: 代码文件数 / 右: 文档文件数) ─────────────
new Chart(document.getElementById('filesChart'), {{
  type: 'line',
  data: {{
    labels: L,
    datasets: [
      {{
        label: '代码文件数',
        data: {cf},
        borderColor: '#fb923c',
        backgroundColor: 'rgba(251,146,60,0.12)',
        fill: true,
        tension: 0.4,
        pointRadius: {pt_radius},
        yAxisID: 'yLeft'
      }},
      {{
        label: '文档文件数',
        data: {df},
        borderColor: '#9aa0a6',
        borderDash: [5, 5],
        fill: false,
        tension: 0.4,
        pointRadius: 0,
        yAxisID: 'yRight'
      }}
    ]
  }},
  options: {{
    ...baseCfg,
    scales: {{
      x: baseCfg.scales.x,
      yLeft: {{
        type: 'linear',
        position: 'left',
        min: 0,
        max: {max_code_files},
        ticks: {{ color: '#fb923c', font: {{ size: 9 }} }},
        grid: {{ color: '#1f2b3a' }},
        title: {{ display: true, text: '代码文件数', color: '#fb923c', font: {{ size: 9 }} }}
      }},
      yRight: {{
        type: 'linear',
        position: 'right',
        min: 0,
        max: {max_doc_files},
        ticks: {{ color: '#9aa0a6', font: {{ size: 9 }} }},
        grid: {{ drawOnChartArea: false }},
        title: {{ display: true, text: '文档文件数', color: '#9aa0a6', font: {{ size: 9 }} }}
      }}
    }},
    plugins: {{ ...baseCfg.plugins, legend: {{ display: true, labels: {{ color: '#9aa0a6', font: {{ size: 10 }} }} }} }}
  }}
}});

// ── ④ 测试用例数 ──────────────────────────────────────────────────────────
new Chart(document.getElementById('testsChart'), {{
  type: 'line',
  data: {{
    labels: L,
    datasets: [{{
      label: '测试用例数',
      data: {tc},
      borderColor: '#34a853',
      backgroundColor: 'rgba(52,168,83,0.12)',
      fill: true,
      tension: 0.4,
      pointRadius: {pt_radius}
    }}]
  }},
  options: baseCfg
}});
</script>
</body>
</html>"""


def main():
    screenshot = parse_args().screenshot

    data = get_data()
    html = build_html(data)
    if html is None:
        print("ERROR: no data collected (dates is empty)", file=sys.stderr)
        sys.exit(1)

    with open(HTML_OUT, "w") as f:
        f.write(html)

    n = len(data["dates"])
    latest = data["dates"][-1]
    print(f"Written: {HTML_OUT}  ({n} days, latest={latest})")

    if screenshot:
        take_screenshot(HTML_OUT)


if __name__ == "__main__":
    main()
