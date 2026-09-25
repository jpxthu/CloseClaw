#!/usr/bin/env bash
# periodic-checks.sh — 定期检查统一入口（T1）
#
# cargo-nextest 只覆盖单元/集成测试；doctest、覆盖率、依赖审计、miri/TSAN
# 等检查此前没有统一入口。本脚本将这些检查集中到一处：
#   每个检查段独立收集退出码，任何一段失败不影响其它段继续执行，
#   最后统一打印汇总表，退出码 = 失败段数量（0 = 全部通过/SKIP）。
#
# 默认配置来自 .config/nextest.toml（slow-timeout 5s / retries 1 / test-threads 24），
# CLI 参数可覆盖（详见该文件注释）。
#
# 工具（cargo-llvm-cov / cargo-deny / cargo-machete / miri / TSAN）缺失时
# 打印安装提示并将该段记为 SKIP，不算失败。
#
# 小范围实测：NEXTEST_EXTRA_ARGS 可透传 nextest 过滤参数，例如
#   NEXTEST_EXTRA_ARGS="-E test(test_exec_)" scripts/periodic-checks.sh --slow
#   NEXTEST_EXTRA_ARGS="-p closeclaw-common" scripts/periodic-checks.sh --flaky
# （按空白分词，不支持引号嵌套）

set -euo pipefail

# ---------- 全局状态 ----------
SECTION_NAMES=()
SECTION_STATUS=()   # PASS / FAIL / SKIP
SECTION_NOTES=()

# 慢用例两档阈值（秒）
SLOW_TIER1=0.1
SLOW_TIER2=1.0
# SLOW 观测线（秒），与 .config/nextest.toml 的 slow-timeout 保持一致
SLOW_MARK_LINE=5.0
# TSAN 目标 triple（按本机平台调整）
TSAN_TARGET="x86_64-unknown-linux-gnu"

# 透传给 nextest 的额外参数（小范围实测用），按空白分词
NEXTEST_EXTRA_ARGS_ARR=()
if [[ -n "${NEXTEST_EXTRA_ARGS:-}" ]]; then
    read -ra NEXTEST_EXTRA_ARGS_ARR <<<"$NEXTEST_EXTRA_ARGS"
fi

usage() {
    cat <<'EOF'
用法: scripts/periodic-checks.sh [选项]

定期检查统一入口。各段独立执行、独立退出码，结束打印汇总表。

选项:
  --slow      慢用例检查（nextest 全量，>0.1s / >1s 两档清单，>5s 标记 SLOW）
  --flaky     不稳定用例检查（nextest --retries 1，汇总 FLAKY 清单；存在 FLAKY 即该段 FAIL）
  --doctest   文档测试（cargo test --workspace --doc --no-fail-fast）
  --coverage  覆盖率（cargo llvm-cov nextest --workspace）
  --deps      依赖检查（cargo-deny check + cargo-machete）
  --heavy     重型检查（miri + TSAN；默认跳过，需显式传入）
  --all       = --slow --flaky --doctest --coverage --deps（不含 --heavy）
  --help, -h  打印本用法

说明:
  工具缺失的段记 SKIP（不算失败），并打印安装提示。
  退出码 = 失败段数量（0 表示全部通过或 SKIP）。
EOF
}

# ---------- 记录与调度 ----------
record_pass() { SECTION_NAMES+=("$1"); SECTION_STATUS+=("PASS"); SECTION_NOTES+=("$2"); }
record_fail() { SECTION_NAMES+=("$1"); SECTION_STATUS+=("FAIL"); SECTION_NOTES+=("$2"); }
record_skip() { SECTION_NAMES+=("$1"); SECTION_STATUS+=("SKIP"); SECTION_NOTES+=("$2"); }

section_header() {
    echo ""
    echo "================================================================"
    echo "== $1"
    echo "================================================================"
}

# require_tool <command> <install-hint>
# 命令存在返回 0；不存在打印安装提示并返回 1（调用方记 SKIP）。
require_tool() {
    local cmd="$1" hint="$2"
    if command -v "$cmd" >/dev/null 2>&1; then
        return 0
    fi
    echo "[SKIP] 未找到 $cmd，请先安装：$hint"
    return 1
}

# run_section <段名> <函数名>
# 段函数返回 0=PASS、77=SKIP（工具缺失）、其它=FAIL；各段互不影响。
run_section() {
    local name="$1" fn="$2"
    section_header "$name"
    local rc=0
    "$fn" || rc=$?
    if [[ $rc -eq 0 ]]; then
        record_pass "$name" "ok"
    elif [[ $rc -eq 77 ]]; then
        record_skip "$name" "工具缺失"
    else
        record_fail "$name" "退出码 $rc"
    fi
    return 0
}

# ---------- --slow：nextest 全量 + 耗时两档清单 ----------
# 说明：libtest-json-plus 的 JSON 走 stdout（进度走 stderr），每条 test 记录
# 带 exec_time；重试过的用例只输出最终一条记录（name 带 #N 后缀）。
do_slow() (
    set -u
    command -v python3 >/dev/null 2>&1 || { echo "[FAIL] 解析 JSON 需要 python3，请先安装"; return 1; }
    local json_out
    json_out=$(mktemp /tmp/periodic-slow.XXXXXX.json)
    trap 'rm -f "$json_out"' EXIT
    local rc=0
    # --no-fail-fast：耗时清单是核心交付物，存在失败用例时继续收集全量数据
    # 再一并报告（真实失败时退出码仍非零，FAIL 语义不变）
    NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 \
        cargo nextest run --workspace --no-fail-fast --message-format libtest-json-plus \
        "${NEXTEST_EXTRA_ARGS_ARR[@]}" \
        >"$json_out" || rc=$?
    if [[ $rc -ne 0 ]]; then
        echo "[WARN] cargo nextest run 退出码 $rc（存在失败用例，--no-fail-fast 已收集全量数据，清单见下）"
    fi
    python3 - "$json_out" "$SLOW_TIER1" "$SLOW_TIER2" "$SLOW_MARK_LINE" <<'PYEOF'
import json, re, sys

path, t1, t2, mark = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
last = {}
for line in open(path, encoding="utf-8", errors="replace"):
    line = line.strip()
    if not line.startswith("{"):
        continue
    try:
        rec = json.loads(line)
    except json.JSONDecodeError:
        continue
    if rec.get("type") != "test" or "exec_time" not in rec:
        continue
    ev = rec.get("event")
    if ev not in ("ok", "failed"):
        continue
    last[rec.get("name", "")] = (rec["exec_time"], ev)

def disp(name):
    # 原始 name 形如 crate::binary$path#attempt：先剥离重试 #N 后缀再转为可读显示
    name = re.sub(r"#\d+$", "", name)
    if "$" in name:
        crate, rest = name.split("$", 1)
        return f"{rest}  [{crate.split('::', 1)[0]}]"
    return name

rows = sorted(last.items(), key=lambda kv: -kv[1][0])
fail_n = sum(1 for _, ev in last.values() if ev == "failed")
tier2 = [(n, v) for n, v in rows if v[0] > t2]
tier1 = [(n, v) for n, v in rows if t1 < v[0] <= t2]
print(f"[slow] 共 {len(rows)} 个用例")
if fail_n:
    print(f"[slow] 含 {fail_n} 个失败用例，清单基于全量数据")
print(f"[slow] >{t2}s（其中 >{mark:g}s 即超 nextest SLOW 观测线）: {len(tier2)} 个")
for n, (t, ev) in tier2:
    slow_tag = " [SLOW]" if t > mark else ""
    print(f"  {t:8.3f}s  {ev:6}  {disp(n)}{slow_tag}")
print(f"[slow] >{t1}s 且 <= {t2}s: {len(tier1)} 个")
for n, (t, ev) in tier1:
    print(f"  {t:8.3f}s  {ev:6}  {disp(n)}")
print(f"[slow] 其余 <= {t1}s: {len(rows) - len(tier1) - len(tier2)} 个（不列出）")
PYEOF
    if [[ $rc -ne 0 ]]; then
        echo "[FAIL] --slow 段判定失败：存在失败用例（退出码 $rc），耗时清单已基于全量数据输出"
        return "$rc"
    fi
    echo "[ok] --slow 检查完成"
    return 0
)

# ---------- --flaky：nextest --retries 1 + FLAKY 清单 ----------
# 说明：重试后转绿的用例在 JSON 中表现为 event=ok 且 name 带 #N（N>=2）后缀，
# 无独立 flaky 字段，据此判定。
do_flaky() (
    set -u
    command -v python3 >/dev/null 2>&1 || { echo "[FAIL] 解析 JSON 需要 python3，请先安装"; return 1; }
    local json_out
    json_out=$(mktemp /tmp/periodic-flaky.XXXXXX.json)
    trap 'rm -f "$json_out"' EXIT
    local rc=0
    # --no-fail-fast：FLAKY 清单是核心交付物，存在失败用例时继续收集全量数据
    # 再一并报告；FLAKY 与真实失败并存时两者各自打印后统一返回非零
    NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 \
        cargo nextest run --workspace --no-fail-fast --retries 1 --message-format libtest-json-plus \
        "${NEXTEST_EXTRA_ARGS_ARR[@]}" \
        >"$json_out" || rc=$?
    if [[ $rc -ne 0 ]]; then
        echo "[WARN] cargo nextest run 退出码 $rc（存在真实失败用例，--no-fail-fast 已收集全量数据，清单见下）"
    fi
    local flaky_rc=0
    python3 - "$json_out" <<'PYEOF' || flaky_rc=$?
import json, re, sys

path = sys.argv[1]
last = {}
for line in open(path, encoding="utf-8", errors="replace"):
    line = line.strip()
    if not line.startswith("{"):
        continue
    try:
        rec = json.loads(line)
    except json.JSONDecodeError:
        continue
    if rec.get("type") != "test" or "exec_time" not in rec:
        continue
    ev = rec.get("event")
    if ev not in ("ok", "failed"):
        continue
    last[rec.get("name", "")] = (rec["exec_time"], ev)

flaky = []
for name, (t, ev) in last.items():
    m = re.search(r"#(\d+)$", name)
    if ev == "ok" and m and int(m.group(1)) >= 2:
        flaky.append((t, re.sub(r"#\d+$", "", name)))

fail_n = sum(1 for _, ev in last.values() if ev == "failed")
print(f"[flaky] 共 {len(last)} 个用例；FLAKY（重试后转绿）: {len(flaky)} 个")
for t, name in sorted(flaky, key=lambda x: -x[0]):
    print(f"  FLAKY  {t:8.3f}s  {name}")
if fail_n:
    print(f"[flaky] 真实失败（重试后仍失败）: {fail_n} 个，明细见 nextest 上方输出")
if flaky:
    sys.exit(1)
PYEOF
    # FLAKY 与真实失败并存时两者各自打印，统一返回非零（失败原因完整呈现）
    local had_fail=0
    if [[ $flaky_rc -ne 0 ]]; then
        echo "[FAIL] 存在 FLAKY 用例（清单见上），建议优先修复"
        had_fail=1
    fi
    if [[ $rc -ne 0 ]]; then
        echo "[FAIL] 存在真实失败用例（nextest 退出码 $rc），清单基于全量数据，明细见上方输出"
        had_fail=1
    fi
    if [[ $had_fail -ne 0 ]]; then
        return 1
    fi
    echo "[ok] --flaky 检查完成：无 FLAKY 用例"
    return 0
)

# ---------- --doctest：文档测试 ----------
do_doctest() (
    set -u
    local rc=0
    cargo test --workspace --doc --no-fail-fast || rc=$?
    if [[ $rc -ne 0 ]]; then
        echo "[FAIL] doctest 存在失败（退出码 $rc），失败明细见上方输出（--no-fail-fast 已尽量汇总）"
        return "$rc"
    fi
    echo "[ok] doctest 全部通过"
    return 0
)

# ---------- --coverage：cargo-llvm-cov ----------
do_coverage() (
    set -u
    require_tool cargo-llvm-cov "cargo install cargo-llvm-cov" || return 77
    local rc=0
    cargo llvm-cov nextest --workspace || rc=$?
    if [[ $rc -ne 0 ]]; then
        echo "[FAIL] cargo llvm-cov nextest 失败（退出码 $rc）"
        return "$rc"
    fi
    echo "[ok] 覆盖率检查完成"
    return 0
)

# ---------- --deps：cargo-deny + cargo-machete ----------
do_deps() (
    set -u
    local fails=0 ran=0
    if command -v cargo-deny >/dev/null 2>&1; then
        ran=1
        local rc=0
        cargo deny check || rc=$?
        if [[ $rc -eq 0 ]]; then
            echo "[ok] cargo-deny check 通过"
        else
            echo "[FAIL] cargo-deny check 失败（退出码 $rc）"
            fails=1
        fi
    else
        echo "[SKIP] 未找到 cargo-deny，请先安装：cargo install cargo-deny"
    fi
    if command -v cargo-machete >/dev/null 2>&1; then
        ran=1
        local rc=0
        cargo machete || rc=$?
        if [[ $rc -eq 0 ]]; then
            echo "[ok] cargo-machete 通过（无未使用依赖）"
        else
            echo "[FAIL] cargo-machete 发现未使用依赖（退出码 $rc），明细见上方"
            fails=1
        fi
    else
        echo "[SKIP] 未找到 cargo-machete，请先安装：cargo install cargo-machete"
    fi
    # 子工具全部缺失 → 整段 SKIP；有跑过的按其结果判定
    if [[ $ran -eq 0 ]]; then
        return 77
    fi
    return "$fails"
)

# ---------- --heavy：miri + TSAN（默认跳过，显式传入） ----------
do_heavy() (
    set -u
    local fails=0 ran=0
    # miri/TSAN 均需 nightly；注意 rustup shim 可能存在但组件未安装，
    # 用功能性命令探测而非 command -v
    if ! rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
        echo "[SKIP] 未找到 nightly 工具链（miri/TSAN 需 nightly：rustup toolchain install nightly）"
        return 77
    fi
    if cargo +nightly miri --version >/dev/null 2>&1; then
        ran=1
        local rc=0
        MIRIFLAGS="-Zmiri-disable-isolation" cargo +nightly miri test --workspace || rc=$?
        if [[ $rc -eq 0 ]]; then
            echo "[ok] miri 通过"
        else
            echo "[FAIL] miri 失败（退出码 $rc）"
            fails=1
        fi
    else
        echo "[SKIP] nightly 未安装 miri 组件（rustup component add miri --toolchain nightly）"
    fi
    if rustup target list --toolchain nightly --installed 2>/dev/null | grep -q "$TSAN_TARGET"; then
        ran=1
        local rc=0
        RUSTFLAGS="-Z sanitizer=thread" \
            cargo +nightly test --workspace --target "$TSAN_TARGET" -- --test-threads=1 || rc=$?
        if [[ $rc -eq 0 ]]; then
            echo "[ok] TSAN 通过"
        else
            echo "[FAIL] TSAN 失败（退出码 $rc）"
            fails=1
        fi
    else
        echo "[SKIP] nightly 未安装目标 $TSAN_TARGET（rustup target add $TSAN_TARGET --toolchain nightly）"
    fi
    # 子项全部缺失 → 整段 SKIP；有跑过的按其结果判定
    if [[ $ran -eq 0 ]]; then
        return 77
    fi
    return "$fails"
)

# ---------- 参数解析 ----------
RUN_SLOW=0 RUN_FLAKY=0 RUN_DOCTEST=0 RUN_COVERAGE=0 RUN_DEPS=0 RUN_HEAVY=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --slow)     RUN_SLOW=1 ;;
        --flaky)    RUN_FLAKY=1 ;;
        --doctest)  RUN_DOCTEST=1 ;;
        --coverage) RUN_COVERAGE=1 ;;
        --deps)     RUN_DEPS=1 ;;
        --heavy)    RUN_HEAVY=1 ;;
        --all)      RUN_SLOW=1; RUN_FLAKY=1; RUN_DOCTEST=1; RUN_COVERAGE=1; RUN_DEPS=1 ;;
        --help|-h)  usage; exit 0 ;;
        *)          echo "未知选项: $1"; echo ""; usage; exit 1 ;;
    esac
    shift
done

if [[ $((RUN_SLOW + RUN_FLAKY + RUN_DOCTEST + RUN_COVERAGE + RUN_DEPS + RUN_HEAVY)) -eq 0 ]]; then
    usage
    exit 1
fi

# ---------- 执行 ----------
echo "定期检查（periodic-checks）"
echo "时间: $(date '+%Y-%m-%d %H:%M:%S')"
echo "工作目录: $(pwd)"

if [[ $RUN_SLOW -eq 1 ]]; then     run_section "--slow 慢用例检查" do_slow; fi
if [[ $RUN_FLAKY -eq 1 ]]; then    run_section "--flaky 不稳定用例检查" do_flaky; fi
if [[ $RUN_DOCTEST -eq 1 ]]; then  run_section "--doctest 文档测试" do_doctest; fi
if [[ $RUN_COVERAGE -eq 1 ]]; then run_section "--coverage 覆盖率" do_coverage; fi
if [[ $RUN_DEPS -eq 1 ]]; then     run_section "--deps 依赖检查" do_deps; fi
if [[ $RUN_HEAVY -eq 1 ]]; then    run_section "--heavy 重型检查（miri/TSAN）" do_heavy; fi

# ---------- 汇总表 ----------
echo ""
echo "================================================================"
echo "== 汇总"
echo "================================================================"
printf '%-4s  %-36s  %-6s  %s\n' "#" "段" "状态" "备注"
printf '%-4s  %-36s  %-6s  %s\n' "--" "------------------------------------" "------" "--------"
PASS_N=0 SKIP_N=0 FAIL_N=0
for i in "${!SECTION_NAMES[@]}"; do
    printf '%-4s  %-36s  %-6s  %s\n' \
        "$((i + 1))" "${SECTION_NAMES[$i]}" "${SECTION_STATUS[$i]}" "${SECTION_NOTES[$i]}"
    case "${SECTION_STATUS[$i]}" in
        PASS) PASS_N=$((PASS_N + 1)) ;;
        SKIP) SKIP_N=$((SKIP_N + 1)) ;;
        FAIL) FAIL_N=$((FAIL_N + 1)) ;;
    esac
done
echo "----------------------------------------------------------------"
echo "共 ${#SECTION_NAMES[@]} 段：PASS=$PASS_N SKIP=$SKIP_N FAIL=$FAIL_N"
if [[ $FAIL_N -gt 0 ]]; then
    echo "存在失败段，退出码 = $FAIL_N"
    exit "$FAIL_N"
fi
echo "全部通过（或 SKIP），退出码 = 0"
exit 0
