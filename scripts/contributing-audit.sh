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
OUT="$(mktemp "${TMPDIR:-/tmp}/contributing-audit.XXXXXX.md")"

# ── 硬限制类：clippy lint 全量（JSON 分组）+ 本脚本自带解析子项 ──
CONF_DIR="$(mktemp -d "${TMPDIR:-/tmp}/clippy-conf.XXXXXX")"
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

CLIPPY_JSON="$(mktemp "${TMPDIR:-/tmp}/clippy-json.XXXXXX")"
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
import json, sys, collections, os, re

clippy_json, out, commit = sys.argv[1], sys.argv[2], sys.argv[3]
ROOT = os.getcwd()

# (code, 标题, 检查方式, 豁免方式, 数据源 provider)。provider 显式声明数据源，
# 不再由 code 前缀隐式推断（避免未来新增 script:: 小节误取 impl 块数据）。
SECTIONS = [
    ("clippy::too_many_lines", "1. 函数体 > 100 行（clippy::too_many_lines, threshold=100）",
     "cargo clippy --workspace --all-targets（clippy.toml: too-many-lines-threshold=100"
     "，由本脚本运行时临时生成）",
     "函数级 `#[allow(clippy::too_many_lines)]` + 理由注释；根治=按阶段拆子函数/抽 match 分支方法",
     "clippy"),
    ("clippy::too_many_arguments", "2. 函数参数 > 6（clippy::too_many_arguments, threshold=6）",
     "cargo clippy --workspace --all-targets（clippy.toml: too-many-arguments-threshold=6"
     "，由本脚本运行时临时生成）",
     "`#[allow(clippy::too_many_arguments)]` + 理由；根治=参数聚合 struct / builder / Option 组合并",
     "clippy"),
    ("clippy::excessive_nesting", "3. 块嵌套 > 3 层（clippy::excessive_nesting, threshold=3）",
     "cargo clippy --workspace --all-targets（clippy.toml: excessive-nesting-threshold=3"
     "，由本脚本运行时临时生成）",
     "`#[allow(clippy::excessive_nesting)]`；注意口径=所有块（含 loop/block），比 CONTRIBUTING 的 match/if 口径严；"
     "根治=提前返回/guard clause/抽函数",
     "clippy"),
    ("clippy::undocumented_unsafe_blocks", "4. unsafe 块缺 // SAFETY: 注释",
     "cargo clippy --force-warn clippy::undocumented_unsafe_blocks",
     "块前补 `// SAFETY: <不变量说明>`（Rustonomicon 引用如适用）",
     "clippy"),
    ("clippy::missing_safety_doc", "5. unsafe fn 缺 /// # Safety 文档",
     "cargo clippy --force-warn clippy::missing_safety_doc",
     "doc 注释补 `# Safety` 段",
     "clippy"),
    ("clippy::disallowed_methods", "6. 禁用方法 set_var/remove_var（load_env_file 场景除外）",
     "cargo clippy --workspace --all-targets（clippy.toml 由本脚本运行时临时生成，"
     "非仓库文件）：disallowed-methods=[std::env::set_var, std::env::remove_var]",
     "唯一豁免点：crates/daemon/src/env_file.rs 的 load_env_file()，按行级 load_env_file 标记文本豁免"
     "（与 CI/pre-commit 同口径：命中行内含 load_env_file 标记即豁免）；其余改参数传递/tempfile",
     "clippy"),
    ("script::impl_block_lines", "7. impl 块行数 > 100（本脚本解析，数据源非 clippy）",
     "python heredoc strip 注释/字符串后花括号配对统计 impl 块首尾行跨度"
     "（inherent 与 trait impl 均计入；文件集合与 B 节一致：src/crates/tests 下 *.rs）",
     "无 lint 豁免，存量如实报告；根治=impl 块按职责拆分到独立文件（feishu plugin.rs 先例）",
     "impl_blocks"),
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
# crates/daemon/src/env_file.rs 的 load_env_file()（行级文本标记豁免，非 #[allow] 属性）。
# 按 fname 缓存整行列表，消除同一文件的重复打开/重复扫描（纯 no-op 重构，取行口径不变）。
_SOURCE_LINES = {}


def source_line(fname, ln):
    lines = _SOURCE_LINES.get(fname)
    if lines is None:
        try:
            with open(fname, errors="replace") as fh:
                lines = fh.readlines()
        except OSError:
            lines = []
        _SOURCE_LINES[fname] = lines
    if 1 <= ln <= len(lines):
        return lines[ln - 1]
    return ""

raw_diags = sum(len(v) for v in groups.values())
groups["clippy::disallowed_methods"] = [
    item for item in groups.get("clippy::disallowed_methods", [])
    if "load_env_file" not in source_line(item[0], item[1])
]

# §7 impl 块行数：自带解析（数据源非 clippy JSON）——strip 注释/字符串内容后
# 按花括号配对统计每个 impl 块首尾行跨度，inherent 与 trait impl 一并计入。
IMPL_CAND_RE = re.compile(
    r"^[ \t]*(?:#\[[^\]\n]*\][ \t]*)*"
    r"(?:pub(?:\s*\([^)\n]*\))?\s+)?(?:unsafe\s+)?impl\b",
    re.M,
)


def strip_rs(text):
    """注释与字符串内容置空（保留换行），供花括号配对使用。"""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            depth, i = 1, i + 2
            while i < n and depth:
                if text[i] == "/" and i + 1 < n and text[i + 1] == "*":
                    depth += 1
                    i += 2
                elif text[i] == "*" and i + 1 < n and text[i + 1] == "/":
                    depth -= 1
                    i += 2
                else:
                    out.append("\n" if text[i] == "\n" else " ")
                    i += 1
            continue
        if c in "rb":  # 原始字符串 r"…" / r#"…"# / br / br#（b 须后随 r）
            j = i + 1
            if c == "b":
                if j >= n or text[j] != "r":
                    out.append(c)
                    i += 1
                    continue
                j += 1
            k = j
            while k < n and text[k] == "#":
                k += 1
            if k < n and text[k] == '"':
                out.append(text[i:k + 1])
                end = text.find('"' + "#" * (k - j), k + 1)
                seg_end = n if end < 0 else end + 1 + (k - j)
                seg = text[k + 1:seg_end]
                out.append("".join(ch if ch == "\n" else " " for ch in seg))
                i = seg_end
                continue
            out.append(c)
            i += 1
            continue
        if c == '"':  # 普通字符串（可跨行），内容置空
            out.append('"')
            i += 1
            while i < n and text[i] != '"':
                if text[i] == "\\" and i + 1 < n:
                    # 转义符与其后字符各置一空格；但 `\`+换行（续行）的换行必须保留，
                    # 否则 strip 后行数少于原文，impl 块起始行/跨度整体漂移。
                    out.append(" \n" if text[i + 1] == "\n" else "  ")
                    i += 2
                    continue
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            if i < n:
                out.append('"')
                i += 1
            continue
        if c == "'":  # 字符字面量置空（'{' 等含花括号），生命周期原样保留
            j = i + 1
            if j < n and text[j] == "\\":
                k = j + 1
                while k < n and text[k] != "'" and text[k] != "\n":
                    k += 1
                if k < n and text[k] == "'":
                    out.append("' '")
                    i = k + 1
                    continue
                out.append("'")
                i += 1
                continue
            if j < n and (text[j].isalpha() or text[j] == "_"):
                k = j
                while k < n and (text[k].isalnum() or text[k] == "_"):
                    k += 1
                if k < n and text[k] == "'":
                    out.append("' '")  # 'a'
                    i = k + 1
                    continue
                out.append("'")  # 'a 生命周期
                i += 1
                continue
            if j + 1 < n and text[j + 1] == "'":
                out.append("' '")  # '#'
                i += 2
                continue
            out.append("'")
            i += 1
            continue
        out.append(c)
        i += 1
    return "".join(out)


def impl_spans(fname):
    """返回 [(impl 起始行, 行数跨度, impl 头文本)]，含配对失败时跳过。"""
    try:
        with open(fname, errors="replace") as fh:
            raw = fh.read()
    except OSError:
        return []
    text = strip_rs(raw)
    # 预检在 strip 后文本上做：与下方 finditer 同一数据源，判定即所用。
    if not IMPL_CAND_RE.search(text):
        return []
    res = []
    for m in IMPL_CAND_RE.finditer(text):
        impl_at = m.start() + m.group(0).rfind("impl")
        open_at = text.find("{", m.end())
        if open_at < 0:
            continue
        depth, close_at = 0, -1
        for p in range(open_at, len(text)):
            ch = text[p]
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    close_at = p
                    break
        if close_at < 0:
            continue
        start_line = text.count("\n", 0, impl_at) + 1
        end_line = text.count("\n", 0, close_at) + 1
        res.append((start_line, end_line - start_line + 1,
                    " ".join(text[impl_at:open_at].split())))
    return res


def collect_impl_violations():
    viol = []
    for root in ("src", "crates", "tests"):
        if not os.path.isdir(root):
            continue
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames.sort()
            for name in sorted(filenames):
                if not name.endswith(".rs"):
                    continue
                fname = os.path.join(dirpath, name)
                for ln, span, header in impl_spans(fname):
                    if span <= 100:
                        continue
                    head = header if len(header) <= 60 else header[:59] + "…"
                    viol.append((fname, ln,
                                 f"impl 块 {span} 行 > 100 行上限（{head}）"))
    return sorted(set(viol))


impl_violations = collect_impl_violations()

# 数据源 provider 显式映射：section 第 5 元素 → 取数函数；未知 provider 直接报错，
# 不再按 code 前缀隐式兜底（防止新增 script:: 小节误取 impl 块违规）。
PROVIDERS = {
    "clippy": lambda _code: sorted(set(groups.get(_code, []))),
    "impl_blocks": lambda _code: impl_violations,
}

with open(out, "w", encoding="utf-8") as f:
    f.write(f"# CONTRIBUTING 违规扫描报告（除单测时长）\n\n> commit: {commit} ｜ 生成: contributing-audit.sh ｜ 耗时项为 clippy 全量\n")
    f.write("> 用法：从各节选条目修复，完成后重跑本脚本验证条目消失。\n\n")
    f.write("## A. 硬限制类（clippy lint + 本脚本自带解析子项）\n\n")
    total_a = 0
    for code, title, how, exempt, provider_name in SECTIONS:
        provider = PROVIDERS.get(provider_name)
        if provider is None:
            raise SystemExit(
                f"未知数据源 provider: {provider_name!r}（section: {code}）")
        items = provider(code)
        total_a += len(items)
        f.write(f"### {title}\n\n- 检查方式：{how}\n- 豁免方式：{exempt}\n- 违规 {len(items)} 条：\n")
        if not items:
            f.write("  - 无违规\n")
        for file, ln, m in items:
            f.write(f"- [ ] `{file}:{ln}` — {m}\n")
        f.write("\n")
    f.write(f"**A 小计：{total_a} 条**\n\n")
print("clippy done:", raw_diags, "diags")
print("impl blocks > 100:", len(impl_violations))
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
