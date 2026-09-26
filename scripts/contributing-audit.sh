#!/usr/bin/env bash
#
# contributing-audit.sh — CONTRIBUTING 违规全量扫描（除单测时长外）
#
# 用途: 输出一份按类别分组的违规清单（临时文件），stdout 打印文件路径 + 前 100 行。
#       供 agent 循环任务消费：从清单中选违规条目处理，修复后重跑脚本验证消失。
# 依赖: cargo + clippy（rustup 自带）、python3、grep
# 耗时: 首次 ~2min（clippy 全量），热缓存 ~1min
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
COMMIT="$(git rev-parse --short HEAD)"
OUT="$(mktemp /tmp/contributing-audit.XXXXXX.md)"

# ── clippy 硬限制类：一次跑全部 lint，JSON 输出后分组 ──────────────
CONF_DIR="$(mktemp -d /tmp/clippy-conf.XXXXXX)"
trap 'rm -rf "$CONF_DIR"' EXIT
cat > "$CONF_DIR/clippy.toml" <<'EOF'
too-many-lines-threshold = 100
too-many-arguments-threshold = 6
excessive-nesting-threshold = 3
disallowed-methods = [
  { path = "std::env::set_var" },
  { path = "std::env::remove_var" },
]
EOF

CLIPPY_JSON="$(mktemp /tmp/clippy-json.XXXXXX)"
CLIPPY_CONF_DIR="$CONF_DIR" cargo clippy --workspace --all-targets --message-format=json -- \
  -A clippy::all -A warnings \
  --force-warn clippy::too_many_lines \
  --force-warn clippy::too_many_arguments \
  --force-warn clippy::excessive_nesting \
  --force-warn clippy::undocumented_unsafe_blocks \
  --force-warn clippy::missing_safety_doc \
  --force-warn clippy::disallowed_methods \
  > "$CLIPPY_JSON" 2>/dev/null || true

python3 - "$CLIPPY_JSON" "$OUT" "$COMMIT" <<'PYEOF'
import json, sys, collections, os

clippy_json, out, commit = sys.argv[1], sys.argv[2], sys.argv[3]
ROOT = os.getcwd()

SECTIONS = [
    ("clippy::too_many_lines", "1. 函数体 > 100 行（clippy::too_many_lines, threshold=100）",
     "cargo clippy --workspace --all-targets（clippy.toml: too-many-lines-threshold=100）",
     "函数级 `#[allow(clippy::too_many_lines)]` + 理由注释；根治=按阶段拆子函数/抽 match 分支方法"),
    ("clippy::too_many_arguments", "2. 函数参数 > 6（clippy::too_many_arguments, threshold=6）",
     "cargo clippy --workspace --all-targets（clippy.toml: too-many-arguments-threshold=6）",
     "`#[allow(clippy::too_many_arguments)]` + 理由；根治=参数聚合 struct / builder / Option 组合并"),
    ("clippy::excessive_nesting", "3. 块嵌套 > 3 层（clippy::excessive_nesting, threshold=3）",
     "cargo clippy --workspace --all-targets（clippy.toml: excessive-nesting-threshold=3）",
     "`#[allow(clippy::excessive_nesting)]`；注意口径=所有块（含 loop/block），比 CONTRIBUTING 的 match/if 口径严；根治=提前返回/guard clause/抽函数"),
    ("clippy::undocumented_unsafe_blocks", "4. unsafe 块缺 // SAFETY: 注释",
     "cargo clippy --force-warn clippy::undocumented_unsafe_blocks",
     "块前补 `// SAFETY: <不变量说明>`（Rustonomicon 引用如适用）"),
    ("clippy::missing_safety_doc", "5. unsafe fn 缺 /// # Safety 文档",
     "cargo clippy --force-warn clippy::missing_safety_doc",
     "doc 注释补 `# Safety` 段"),
    ("clippy::disallowed_methods", "6. 禁用方法 set_var/remove_var（load_env_file 场景除外）",
     "cargo clippy --workspace --all-targets（clippy.toml 由本脚本运行时临时生成，非仓库文件）：disallowed-methods=[std::env::set_var, std::env::remove_var]",
     "唯一豁免点：crates/daemon/src/mod.rs 的 load_env_file()，按行级 load_env_file 标记文本豁免（与 CI/pre-commit 同口径：命中行内含 load_env_file 标记即豁免）；其余改参数传递/tempfile"),
]

groups = collections.defaultdict(list)
for line in open(clippy_json, errors="replace"):
    try:
        msg = json.loads(line)
    except json.JSONDecodeError:
        continue
    diag = msg.get("message") or {}
    code = (diag.get("code") or {}).get("code", "")
    if not code.startswith("clippy::"):
        continue
    span = next((s for s in diag.get("spans", []) if s.get("is_primary")), None)
    if not span:
        continue
    fname = span["file_name"]
    if fname.startswith(ROOT + "/"):
        fname = fname[len(ROOT) + 1:]
    key = (fname, span["line_start"], code, diag.get("message", ""))
    groups[code].append((fname, span["line_start"], diag.get("message", "")))

# §6 行级豁免：与 CI/pre-commit（scripts/check-env-var.sh）同口径——命中行内含 load_env_file
# 标记即豁免。clippy 侧无法表达行级豁免，故在此后处理环节过滤；豁免点为
# crates/daemon/src/mod.rs 的 load_env_file()（行级文本标记豁免，非 #[allow] 属性）。
def source_line(fname, ln):
    try:
        with open(fname, errors="replace") as fh:
            for i, line in enumerate(fh, 1):
                if i == ln:
                    return line
    except OSError:
        pass
    return ""

raw_diags = sum(len(v) for v in groups.values())
groups["clippy::disallowed_methods"] = [
    item for item in groups.get("clippy::disallowed_methods", [])
    if "load_env_file" not in source_line(item[0], item[1])
]

with open(out, "w", encoding="utf-8") as f:
    f.write(f"# CONTRIBUTING 违规扫描报告（除单测时长）\n\n> commit: {commit} ｜ 生成: contributing-audit.sh ｜ 耗时项为 clippy 全量\n")
    f.write("> 用法：从各节选条目修复，完成后重跑本脚本验证条目消失。\n\n")
    f.write("## A. clippy 硬限制类\n\n")
    total_a = 0
    for code, title, how, exempt in SECTIONS:
        items = sorted(set(groups.get(code, [])))
        total_a += len(items)
        f.write(f"### {title}\n\n- 检查方式：{how}\n- 豁免方式：{exempt}\n- 违规 {len(items)} 条：\n")
        if not items:
            f.write("  - 无违规\n")
        for file, ln, m in items:
            f.write(f"- [ ] `{file}:{ln}` — {m}\n")
        f.write("\n")
    f.write(f"**A 小计：{total_a} 条**\n\n")
print("clippy done:", raw_diags, "diags")
PYEOF
rm -f "$CLIPPY_JSON"

# ── B. 文件行数 > 1000 ─────────────────────────────────────────────
{
echo "## B. 文件行数 > 1000"
echo
echo "- 检查方式：\`find src crates tests -name '*.rs' | xargs wc -l\`（含测试文件）"
echo "- 豁免方式：无 lint 豁免；pre-commit hook 只查 staged 文件（未改动的历史文件不拦），根治=拆子模块"
echo "- 违规："
} >> "$OUT"
N=$(find src crates tests -name '*.rs' -exec awk 'END{if(NR>1000) print FILENAME": "NR" 行"}' {} \; | tee -a "$OUT" | wc -l)
[ "$N" -eq 0 ] && echo "  - 无违规" >> "$OUT"
echo "" >> "$OUT"

# ── C. 单行宽度 > 100 ─────────────────────────────────────────────
{
echo "## C. 单行宽度 > 100 字符"
echo
echo "- 检查方式：grep 全量 .rs（含注释/字符串行；rustfmt 折不动的长字符串属已知豁免类）"
echo "- 豁免方式：长字符串/URL/SQL 字面量等 rustfmt 无法折行内容可放过；代码结构行必须拆"
echo "- 违规（按文件聚合，括号内为行号）："
} >> "$OUT"
grep -rnE '.{101,}' src crates --include='*.rs' 2>/dev/null | \
  python3 -c "
import sys, collections
byfile = collections.defaultdict(list)
for ln in sys.stdin:
    f, n, _ = ln.split(':', 2)
    byfile[f].append(n)
for f, ns in sorted(byfile.items()):
    print(f'- [ ] \`{f}\`（{len(ns)} 行: {\", \".join(ns[:10])}{\"…\" if len(ns)>10 else \"\"}）')
" >> "$OUT" || echo "  - 无违规" >> "$OUT"
echo "" >> "$OUT"

# ── D. mod.rs 含实质定义 ───────────────────────────────────────────
{
echo "## D. mod.rs 含实质代码（应只放 pub use / pub mod）"
echo
echo "- 检查方式：mod.rs 中 grep fn/struct/enum/impl/trait 定义"
echo "- 豁免方式：无；根治=定义下沉子模块，mod.rs 只留 re-export"
echo "- 违规："
} >> "$OUT"
FOUND=0
while IFS= read -r f; do
  C=$(grep -cE '^[[:space:]]*(pub )?(async )?fn |^[[:space:]]*(pub )?struct |^[[:space:]]*(pub )?enum |^[[:space:]]*impl |^[[:space:]]*(pub )?trait ' "$f" || true)
  [ "$C" -gt 0 ] && { echo "- [ ] \`$f\`（$C 处定义）" >> "$OUT"; FOUND=1; }
done < <(find src crates -name mod.rs)
[ "$FOUND" -eq 0 ] && echo "  - 无违规" >> "$OUT"
echo "" >> "$OUT"

# ── E. 依赖分层违规 ────────────────────────────────────────────────
{
echo "## E. 依赖分层违规（横向/逆向/未登记）"
echo
echo "- 检查方式：解析 crates/*/Cargo.toml 的 closeclaw-* 依赖，比对 CONTRIBUTING 分层表"
echo "  L0=common,platform L1=config,tasks L2=llm,session,permission L3=processor_chain,im_adapter,tools,skills,system_prompt,slash,memory L4=agent,gateway L5=daemon"
echo "- 豁免方式：脚本顶部 EXEMPT_EDGES 数组（格式 src->dst）；根治=概念下沉 common trait 或上移消费方"
echo "- 违规："
} >> "$OUT"
EXEMPT_EDGES=""  # 豁免清单：空格分隔，如 "slash->gateway im_adapter->gateway"；分层现状豁免待 owner 定夺后填入
export EXEMPT_EDGES
python3 - >> "$OUT" <<'PYEOF'
import os, re, sys

ROOT = os.getcwd()
LAYER = {"common":0,"platform":0,"config":1,"tasks":1,"llm":2,"session":2,"permission":2,
         "processor_chain":3,"im_adapter":3,"tools":3,"skills":3,"system_prompt":3,"slash":3,"memory":3,
         "agent":4,"gateway":4,"daemon":5}
exempt = set(os.environ.get("EXEMPT_EDGES","").split())
viol = []
for d in sorted(os.listdir("crates")):
    toml = f"crates/{d}/Cargo.toml"
    if not os.path.isfile(toml): continue
    deps = []
    in_dep = False
    for ln in open(toml):
        s = ln.strip()
        if re.match(r'^\[', s):
            in_dep = s.startswith("[dependencies]")
            continue
        if in_dep:
            m = re.match(r'^closeclaw-([a-z_]+)\s*=', s)
            if m: deps.append(m.group(1))
    sl = LAYER.get(d)
    for dep in deps:
        tl = LAYER.get(dep)
        edge = f"{d}->{dep}"
        if edge in exempt: continue
        if tl is None:
            viol.append(f"- [ ] `{d}` → `{dep}`（**目标 crate 未登记层级**）")
        elif sl is None:
            viol.append(f"- [ ] `{d}`（**自身未登记层级**）→ `{dep}`")
        elif sl == tl and sl >= 2:
            viol.append(f"- [ ] `{d}` → `{dep}`（L{sl} 横向依赖，禁止）")
        elif tl > sl:
            viol.append(f"- [ ] `{d}` → `{dep}`（L{sl}→L{tl} 逆向依赖，禁止）")
print("\n".join(viol) if viol else "  - 无违规")
PYEOF
echo "" >> "$OUT"

# ── F. crates/ ↔ docs/design/ 对应 ────────────────────────────────
{
echo "## F. crate 缺设计文档（docs/design/<name>/）"
echo
echo "- 检查方式：crates/ 目录与 docs/design/ 目录 diff"
echo "- 豁免方式：无；根治=补 docs/design/<name>/（含 fake_llm 等测试基础设施 crate）"
echo "- 违规："
} >> "$OUT"
FOUND=0
for d in crates/*/; do
  n=$(basename "$d")
  [ -d "docs/design/$n" ] || { echo "- [ ] \`crates/$n\` 无 docs/design/$n/" >> "$OUT"; FOUND=1; }
done
[ "$FOUND" -eq 0 ] && echo "  - 无违规" >> "$OUT"
echo "" >> "$OUT"

# ── G. 测试红线（静态抽查类）──────────────────────────────────────
{
echo "## G. 测试红线（静态抽查口径）"
echo
echo "### G1. 测试上下文 sleep（thread::sleep / tokio sleep ≥ 等待事件）"
echo
echo "- 检查方式：grep 测试文件（tests/、*_tests.rs、tests.rs、#[cfg(test)] 文件名启发）中的 sleep 调用"
echo "- 豁免方式：注释说明的「模拟时间流逝/退避」可豁免；等待事件类必须改 tokio::time::pause/advance 或注入 Clock"
echo "- 违规（需人工区分等待/流逝）："
} >> "$OUT"
find src crates tests -name '*.rs' | grep -E 'tests/|_tests\.rs|/tests\.rs|_tests/' | xargs grep -nE 'thread::sleep|time::sleep' 2>/dev/null | \
  sed 's/^\([^:]*\):\([0-9]*\):.*/- [ ] `\1:\2`/' | sort -u >> "$OUT" || echo "  - 无违规" >> "$OUT"
{
echo
echo "### G2. bind 硬编码端口（应 port 0）"
echo
echo "- 检查方式：grep bind 调用含字面量非零端口"
echo "- 豁免方式：无；改 (\"127.0.0.1\", 0) + local_addr() 取实际端口"
echo "- 违规："
} >> "$OUT"
grep -rnE '(bind|Bind)\([^)]*"(localhost|127\.0\.0\.1|0\.0\.0\.0|::1)?:(0*[1-9][0-9]{0,4})"' src crates tests --include='*.rs' 2>/dev/null | \
  sed 's/^\([^:]*\):\([0-9]*\):.*/- [ ] `\1:\2`/' | sort -u >> "$OUT" || echo "  - 无违规" >> "$OUT"
{
echo
echo "### G3. 测试写盘未走 TempDir（抽查）"
echo
echo "- 检查方式：grep 测试上下文 std::env::temp_dir() 手拼路径与 \"/tmp/\" 字面量写盘"
echo "- 豁免方式：tempfile::TempDir 管理生命周期；固定子目录必须改 TempDir（并行互踩+残留）"
echo "- 违规（需人工确认是否真实写盘）："
} >> "$OUT"
find src crates tests -name '*.rs' | grep -E 'tests/|_tests\.rs|/tests\.rs|_tests/' | xargs grep -n 'env::temp_dir()\|"/tmp/' 2>/dev/null | \
  sed 's/^\([^:]*\):\([0-9]*\):.*/- [ ] `\1:\2`/' | sort -u | head -50 >> "$OUT" || echo "  - 无违规" >> "$OUT"
echo "" >> "$OUT"

# ── 收尾：计数 + stdout ────────────────────────────────────────────
TOTAL=$(grep -c '^- \[ \]' "$OUT" || true)
{
echo "---"
echo "**总计待处理条目：$TOTAL**（clippy 类为精确口径；G 类为抽查口径，需人工确认）"
} >> "$OUT"

echo ""
echo "=============================================="
echo "报告: $OUT"
echo "条目: $TOTAL"
echo "=============================================="
head -100 "$OUT"
