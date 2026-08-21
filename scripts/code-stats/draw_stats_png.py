#!/usr/bin/env python3
"""
Generate a PNG chart for CloseClaw daily stats (matplotlib backend).

Replaces the legacy Chart.js + headless-Chrome pipeline. All four panels share
one X axis (vertical alignment is exact by construction); output is a PNG
rendered headlessly by Agg — no browser, no CDN, no PIL crop.

Usage:
    python3 draw_stats_png.py                  # collect + render PNG
    python3 draw_stats_png.py --no-cache       # bypass data/cache.json
    python3 draw_stats_png.py --jsonl FILE     # render from saved JSONL

Dependencies: matplotlib (pip install matplotlib)
              Noto Sans CJK for Chinese labels (auto-detected; falls back
              gracefully if missing)

Output:
    scripts/code-stats/code_stats_chart.png  (2000x1600, 2x scale)
"""

import argparse
import json
import sys
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.dates as mdates
from matplotlib import font_manager
from datetime import datetime

# Import data collector
sys.path.insert(0, str(Path(__file__).parent.resolve()))
from collect_code_stats import get_data

SCRIPT_DIR = Path(__file__).parent.resolve()
PNG_OUT = SCRIPT_DIR / "code_stats_chart.png"

# Dark theme palette (kept from legacy chart)
BG_FIG = "#0f1923"
BG_AX = "#1a2332"
FG = "#e8eaed"
GRID = "#1f2b3a"
TICK = "#5f6b7a"
BLUE = "#4285f4"
RED = "#ea4335"
CYAN = "#46bdc6"
ORANGE = "#fb923c"
GRAY = "#9aa0a6"
GREEN = "#34a853"


def setup_cjk_font():
    """Prefer Noto Sans CJK SC; register user-local fonts if not yet known."""
    want = ["Noto Sans CJK SC", "Noto Sans SC", "WenQuanYi Zen Hei", "WenQuanYi Micro Hei"]

    def _try_install():
        names = {f.name for f in font_manager.fontManager.ttflist}
        for name in want:
            if name in names:
                plt.rcParams["font.family"] = ["sans-serif"]
                plt.rcParams["font.sans-serif"] = [name, "DejaVu Sans"]
                return name
        return None

    hit = _try_install()
    if hit:
        return hit

    # Register from user-local fonts dir, then retry.
    user_fonts = Path.home() / ".local/share/fonts"
    if user_fonts.exists():
        for p in list(user_fonts.glob("*CJK*.tt[cf]")) + list(user_fonts.glob("NotoSansSC*.otf")):
            try:
                font_manager.fontManager.addfont(str(p))
            except Exception:
                pass
    return _try_install()


def forward_fill(arr):
    """Replace None with the previous non-None value (initial Nones -> 0)."""
    result, last = [], 0
    for v in arr:
        if v is not None:
            last = v
        result.append(last)
    return result


def fmt_k(v, _pos=None):
    """Format a tick value as compact "k" notation (12.3k / 1.2k / 900)."""
    if v >= 10000:
        return f"{v/1000:.0f}k"
    if v >= 1000:
        return f"{v/1000:.1f}k"
    return f"{v:.0f}"


# (start, end, label) — short-span milestones (start/end inclusive)
MILESTONES = [
    ("2026-04-10", "2026-04-10", "最后一条 issue 实现 commit，issue-driven 终结"),
    ("2026-05-15", "2026-05-17", "design-doc 体系奠基：初始化 + 两天铺开 45 个模块文档"),
    ("2026-06-27", "2026-06-27", "拆 crates / Cargo workspace 重构（±峰值周）"),
    ("2026-07-07", "2026-07-11", "requirement-doc 体系奠基：四模块需求文档成型"),
    ("2026-08-11", "2026-08-11", "系统迁移：双核阿里云 → 本地 WSL2"),
]

MS_COLORS = ["#8ab4f8", "#81c995", "#fdd663", "#c58fff", "#f6aea9"]


def draw_milestones(fig, axes, dates):
    """Narrow colored point markers in the top axis, no text on chart."""
    from datetime import datetime as _dt
    from datetime import timedelta as _td
    dmin, dmax = min(dates), max(dates)
    ax_top = axes[0]
    for i, (ds, de, _label) in enumerate(MILESTONES):
        d = _dt.strptime(ds, "%Y-%m-%d")
        d1 = _dt.strptime(de, "%Y-%m-%d")
        if d1 < dmin or d > dmax:
            continue
        color = MS_COLORS[i % len(MS_COLORS)]
        d0c, d1c = max(d - _td(days=1), dmin), min(d1 + _td(days=1), dmax)
        ax_top.axvspan(d0c, d1c, color=color,
                       alpha=0.5, zorder=0, lw=0)


def milestone_legend(fig):
    """Text box on the right side listing milestone dates + events."""
    lines = ["里程碑", ""]
    for i, (ds, de, label) in enumerate(MILESTONES):
        span = ds[5:] if ds == de else f"{ds[5:]} ~ {de[5:]}"
        lines.append(f"{span}  {label}")
    txt = "\n".join(lines)
    fig.text(
        0.995, 0.60, txt,
        ha="left", va="top", fontsize=7.8, color="#b4c4d4",
        linespacing=1.9,
        bbox=dict(boxstyle="round,pad=0.6", facecolor="#1a2332",
                  edgecolor="#2a3a50", linewidth=0.8),
    )


def load_weekly_add_del(path):
    """Load weekly additions/deletions JSONL -> ([datetime], [add], [del])."""
    p = Path(path)
    if not p.exists():
        return [], [], []
    from datetime import datetime as _dt
    ds, adds, dels = [], [], []
    for line in p.read_text().splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        ds.append(_dt.strptime(r["week"], "%Y-%m-%d"))
        adds.append(int(r["add"]))
        dels.append(int(r["del"]))
    return ds, adds, dels


def load_jsonl(path):
    """Optional: render from a saved JSONL export instead of live git walk."""
    rows = [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]
    n = len(rows)
    return {
        "dates": [r.get("date") or r.get("dates") for r in rows],
        "code_total_loc": [r.get("code_total_loc") for r in rows],
        "code_changed_cum": [r.get("code_changed_cum") for r in rows],
        "doc_total_loc": [r.get("doc_total_loc") for r in rows],
        "code_files": [r.get("code_files") for r in rows],
        "doc_files": [r.get("doc_files") for r in rows],
        "test_cases": [r.get("test_cases") for r in rows],
        "test_loc": [r.get("test_loc") for r in rows],
        "n_days": n,
    }


def build_fig(data):
    """Build the 5-panel dark-theme figure from ``data`` (get_data() shape).

    Panels: ① code LOC (dual axis, cum churn right) ①b weekly +/- bars
    ② doc LOC ③ file counts (dual axis) ④ test cases. All panels share
    the X axis. Milestone spans are drawn inside panel ① only; the side
    legend panel lists them. Returns the figure, or None for empty data.
    """
    dates = [datetime.strptime(d, "%Y-%m-%d") for d in data["dates"]]
    if not dates:
        return None

    code_loc = forward_fill(data["code_total_loc"])
    code_cum = forward_fill(data["code_changed_cum"])
    doc_loc = forward_fill(data["doc_total_loc"])
    code_files = forward_fill(data["code_files"])
    doc_files = forward_fill(data["doc_files"])
    tests = forward_fill(data["test_cases"])
    test_loc = forward_fill(data.get("test_loc", []))

    font = setup_cjk_font()

    fig, axes = plt.subplots(
        5, 1, figsize=(10, 10), sharex=True,
        gridspec_kw={"hspace": 0.5, "top": 0.90},
    )
    fig.patch.set_facecolor(BG_FIG)

    def _style_ax(ax, title):
        """Apply dark-theme styling to one panel (face, ticks, grid)."""
        ax.set_facecolor(BG_AX)
        ax.set_title(title, color="#b4c4d4", fontsize=8.5, loc="left", pad=6)
        ax.tick_params(colors=TICK, labelsize=7)
        ax.grid(True, color=GRID, linewidth=0.6)
        for s in ax.spines.values():
            s.set_color(GRID)

    def _fmt_all_y(ax):
        """Apply the compact-k formatter to an axis' Y ticks."""
        ax.yaxis.set_major_formatter(plt.FuncFormatter(fmt_k))

    # ── ① 代码行数: 左轴 代码总行数/测试代码, 右轴 累计改动 ─────────────
    ax = axes[0]
    _style_ax(ax, "① 代码行数")
    ln1, = ax.plot(dates, code_loc, color=BLUE, lw=1.4, label="代码总行数")
    ax.fill_between(dates, code_loc, color=BLUE, alpha=0.10)
    ln3, = ax.plot(dates, test_loc, color=BLUE, lw=1.0, ls=(0, (6, 4)), label="测试代码行数")
    axr = ax.twinx()
    axr.set_facecolor("none")
    ln2, = axr.plot(dates, code_cum, color=RED, lw=1.2, ls=(0, (5, 5)), label="累计改动")
    ax.set_ylabel("代码行数", color=BLUE, fontsize=7.5)
    axr.set_ylabel("累计改动", color=RED, fontsize=7.5)
    axr.tick_params(colors=RED, labelsize=7)
    for s in axr.spines.values():
        s.set_color(GRID)
    axr.grid(False)
    _fmt_all_y(ax)
    axr.yaxis.set_major_formatter(plt.FuncFormatter(fmt_k))
    ax.legend(handles=[ln1, ln3, ln2], loc="upper left", fontsize=6.5,
              facecolor=BG_AX, edgecolor=GRID, labelcolor=GRAY)

    # ── ①b 周代码频度: additions 上 / deletions 下 (GitHub style) ───────
    ax = axes[1]
    _style_ax(ax, "①b 周代码频度 (+/-)")
    wd, wadd, wdel = load_weekly_add_del(SCRIPT_DIR / "data" / "weekly_add_del.jsonl")
    if wd:
        ax.bar(wd, wadd, width=5.5, color="#2ea043", alpha=0.85, label="additions")
        ax.bar(wd, [-v for v in wdel], width=5.5, color="#e5534b", alpha=0.85, label="deletions")
        ax.axhline(0, color=FG, lw=0.7)
        ax.set_ylabel("行数", color=FG, fontsize=7.5)
        ax.yaxis.set_major_formatter(plt.FuncFormatter(fmt_k))
        ax.legend(loc="upper right", fontsize=6.5, facecolor=BG_AX, edgecolor=GRID, labelcolor=GRAY)
        hi = max(wadd); lo = max(wdel)
        pad = (hi + lo) * 0.10
        ax.set_ylim(-(lo + pad), hi + pad)
    else:
        ax.text(0.5, 0.5, "no weekly_add_del.jsonl — run collect_weekly_add_del.py",
                transform=ax.transAxes, ha="center", va="center", color=TICK, fontsize=7)

    # ── ② 文档行数 ──────────────────────────────────────────────────────
    ax = axes[2]
    _style_ax(ax, "② 文档行数")
    ax.plot(dates, doc_loc, color=CYAN, lw=1.4, label="文档总行数")
    ax.fill_between(dates, doc_loc, color=CYAN, alpha=0.10)
    ax.set_ylabel("行数", color=CYAN, fontsize=7.5)
    _fmt_all_y(ax)
    ax.legend(loc="upper left", fontsize=6.5, facecolor=BG_AX,
              edgecolor=GRID, labelcolor=GRAY)

    # ── ③ 源文件数: 左轴 代码文件数, 右轴 文档文件数 ────────────────────
    ax = axes[3]
    _style_ax(ax, "③ 源文件数")
    ln1, = ax.plot(dates, code_files, color=ORANGE, lw=1.4, label="代码文件数")
    ax.fill_between(dates, code_files, color=ORANGE, alpha=0.10)
    axr = ax.twinx()
    axr.set_facecolor("none")
    ln2, = axr.plot(dates, doc_files, color=GRAY, lw=1.2, ls=(0, (5, 5)), label="文档文件数")
    ax.set_ylabel("代码文件数", color=ORANGE, fontsize=7.5)
    axr.set_ylabel("文档文件数", color=GRAY, fontsize=7.5)
    axr.tick_params(colors=GRAY, labelsize=7)
    for s in axr.spines.values():
        s.set_color(GRID)
    axr.grid(False)
    _fmt_all_y(ax)
    axr.yaxis.set_major_formatter(plt.FuncFormatter(fmt_k))
    ax.legend(handles=[ln1, ln2], loc="upper left", fontsize=6.5,
              facecolor=BG_AX, edgecolor=GRID, labelcolor=GRAY)

    # ── ④ 测试用例数 ────────────────────────────────────────────────────
    ax = axes[4]
    _style_ax(ax, "④ 测试用例数")
    ax.plot(dates, tests, color=GREEN, lw=1.4, label="测试用例数")
    ax.fill_between(dates, tests, color=GREEN, alpha=0.10)
    ax.set_ylabel("用例数", color=GREEN, fontsize=7.5)
    _fmt_all_y(ax)
    ax.legend(loc="upper left", fontsize=6.5, facecolor=BG_AX,
              edgecolor=GRID, labelcolor=GRAY)

    # Shared X axis formatting
    axes[4].xaxis.set_major_locator(mdates.MonthLocator())
    axes[4].xaxis.set_major_formatter(mdates.DateFormatter("%m-%d"))
    axes[4].xaxis.set_minor_locator(mdates.WeekdayLocator(byweekday=mdates.SA))
    for label in axes[4].get_xticklabels():
        label.set_color(TICK)

    # Header
    n = len(dates)
    fig.suptitle(
        "CloseClaw — Master 分支每日代码统计",
        color="#8ab4f8", fontsize=13, y=0.975,
    )
    fig.text(
        0.5, 0.925,
        f"{data['dates'][0]} → {data['dates'][-1]}  |  {n} 天  |  测试虾"
        + ("" if font else "  [no CJK font]"),
        ha="center", color=TICK, fontsize=8,
    )

    draw_milestones(fig, axes, dates)
    milestone_legend(fig)
    return fig


def main():
    """CLI entry: collect (or load) data, render the PNG, optionally dump."""
    parser = argparse.ArgumentParser(description="Render CloseClaw code stats chart (matplotlib).")
    parser.add_argument("--jsonl", help="render from JSONL export instead of live git walk")
    parser.add_argument("--no-cache", action="store_true",
                        help="bypass data/cache.json (full re-collect)")
    parser.add_argument("--out", default=str(PNG_OUT), help="output PNG path")
    parser.add_argument("--dump-jsonl", help="also dump collected data to this JSONL path")
    args = parser.parse_args()

    if args.jsonl:
        data = load_jsonl(args.jsonl)
    else:
        data = get_data(use_cache=not args.no_cache)
    if not data["dates"]:
        print("ERROR: no data (dates is empty)", file=sys.stderr)
        sys.exit(1)

    if args.dump_jsonl:
        keys = ["dates", "code_total_loc", "code_changed_cum", "doc_total_loc",
                "code_files", "doc_files", "test_cases", "test_loc"]
        with open(args.dump_jsonl, "w") as f:
            for i in range(len(data["dates"])):
                f.write(json.dumps({k: data[k][i] for k in keys}) + "\n")
        print(f"Data dumped: {args.dump_jsonl}")

    fig = build_fig(data)
    if fig is None:
        print("ERROR: empty data", file=sys.stderr)
        sys.exit(1)

    fig.savefig(args.out, dpi=200, facecolor=fig.get_facecolor(),
                bbox_inches="tight", pad_inches=0.15)
    plt.close(fig)
    print(f"Written: {args.out}  ({len(data['dates'])} days)")


if __name__ == "__main__":
    main()
