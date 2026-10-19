#!/usr/bin/env bash
#
# check-line-width.sh — 100 字符行宽硬限检查（staged 增量口径，pre-commit 专用）
#
# 背景: CONTRIBUTING.md「硬性限制」规定单行宽度 100 字符，但 cargo fmt 默认
#       wrap_comments=false，注释行（//、///、//!）不经 fmt 检查；Cargo.toml 则
#       完全不在 fmt 管辖内。本脚本补齐这两类内容的行宽强制拦截。
# 用法: bash scripts/check-line-width.sh [staged]
#         staged 只判定 staged 新增行（默认模式），pre-commit 用
# 范围: staged（diff-filter ACMR）中 basename 为 *.rs 与 Cargo.toml 的文件，
#       仅检查新增（+）行，不背负历史技术债。.rs 只查注释行（允许前导空白），
#       代码行宽由 cargo fmt 负责；Cargo.toml 查全部新增行（无 fmt 管辖）。
# 口径: 行宽按字符（Unicode code point）计，非字节，与 CONTRIBUTING「100 字符」
#       字面语义及 rustfmt max_width 一致。
# 退出码: 0=通过, 1=超宽命中, 2=用法错误 / 非 git 仓库 / 无可用 UTF-8 locale
#
# 细则: CONTRIBUTING.md「硬性限制」（单行宽度 100 字符）
set -euo pipefail

# 行宽上限：与 CONTRIBUTING.md「硬性限制」、rustfmt max_width 默认值一致
readonly MAX_WIDTH=100

# 超宽判定程序（单一来源）：解析 unified diff，输出「新文件行号:新增行内容」。
# hunk 头重置新文件起始行号，逐 @@ 独立记账（不跨 hunk 累加）。
AWK_PROG='
substr($0, 1, 2) == "@@" {
    if (match($0, /[+][0-9]+(,[0-9]+)?/)) {
        split(substr($0, RSTART + 1, RLENGTH - 1), a, ",")
        newln = a[1] + 0
    }
    in_hunk = 1
    next
}
# 跳过条件仅 `+++ `（空格）形态（与 check-env-var.sh 同口径）：文件头行不参与
# 判定与行号记账；hunk 内以 `+++ ` 起始的新增行（内容以 `++ ` 起始）不判但记账
substr($0, 1, 4) == "+++ " {
    if (in_hunk) newln++
    next
}
substr($0, 1, 1) == "+" {
    if (in_hunk) printf "%d:%s\n", newln, substr($0, 2)
    if (in_hunk) newln++
    next
}
substr($0, 1, 1) == " " {
    if (in_hunk) newln++
    next
}
# 其余行（`-` 删除行、`\ No newline` 标记、diff 头等）：不计入新文件，不推进
'

# 探测可用 UTF-8 locale：字符计数依赖 locale（C locale 按字节，中文 1 字符=3 字节
# 会被误拦）。全部候选不可用则拒绝执行（fail closed，宁可不检也不按字节误判）。
pick_utf8_locale() {
    local loc
    for loc in C.UTF-8 C.utf8 en_US.UTF-8 en_US.utf8; do
        if LC_ALL="$loc" bash -c 'p=中中中; [ "${#p}" -eq 3 ]' 2>/dev/null; then
            printf '%s' "$loc"
            return 0
        fi
    done
    return 1
}

if ! utf8_locale="$(pick_utf8_locale)"; then
    echo "ERROR: 未找到可用的 UTF-8 locale，无法保证行宽按字符计数，拒绝执行" >&2
    echo "修复：安装 C.UTF-8 locale（如 locale-gen C.UTF-8）后重试" >&2
    exit 2
fi
export LC_ALL="$utf8_locale"

# staged 中超宽新增行的收集：basename *.rs 只查注释行（允许前导空白），
# basename Cargo.toml 查全部新增行
collect_staged() {
    local staged_files f base kind records rec lineno content trimmed
    staged_files=$(git diff --cached --name-only --diff-filter=ACMR)
    if [ -z "$staged_files" ]; then
        return 0
    fi
    while IFS= read -r f; do
        if [ -z "$f" ]; then
            continue
        fi
        base=$(basename "$f")
        kind=""
        case "$base" in
            *.rs) kind=rs ;;
            Cargo.toml) kind=toml ;;
            *) continue ;;
        esac
        # diff 形态与 check-env-var.sh 一致（git 默认上下文），上下文行参与行号记账
        records=$(git diff --cached --diff-filter=ACMR --no-color -- "$f" \
            | awk "$AWK_PROG" || true)
        while IFS= read -r rec; do
            if [ -z "$rec" ]; then
                continue
            fi
            lineno="${rec%%:*}"
            content="${rec#*:}"
            if [ "$kind" = rs ]; then
                trimmed="${content#"${content%%[![:space:]]*}"}"
                case "$trimmed" in
                    //*) ;;
                    *) continue ;;
                esac
            fi
            if [ "${#content}" -gt "$MAX_WIDTH" ]; then
                printf '%s:%s:%d\n' "$f" "$lineno" "${#content}"
            fi
        done <<< "$records"
    done <<< "$staged_files"
}

# 统一错误文案（唯一来源：超宽清单 + 分类修复指引）
print_failure() {
    echo "ERROR: 以下 staged 新增行超过 ${MAX_WIDTH} 字符行宽上限（按字符计，非字节）："
    echo "$1"
    echo ""
    echo "修复指引（CONTRIBUTING.md「硬性限制」：单行宽度 ${MAX_WIDTH} 字符）："
    echo "  - .rs 注释行（//、///、//!）：拆分为多行注释；代码行格式与行宽由 cargo fmt 负责改写"
    echo "  - Cargo.toml：改写为 toml 多行语法（如依赖数组逐项一行）"
}

usage() {
    echo "usage: $0 [staged]" >&2
}

if [ "$#" -gt 1 ]; then
    usage
    exit 2
fi
mode="${1:-staged}"
case "$mode" in
    staged) ;;
    *)
        usage
        exit 2
        ;;
esac

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "ERROR: $0 必须在 git 仓库内运行（无法执行行宽检查）" >&2
    exit 2
fi

# 定位到仓库根：消除「CWD=仓库根」隐含前提（与 check-env-var.sh 同口径）
cd "$(git rev-parse --show-toplevel)"

violations="$(collect_staged)"
if [ -n "$violations" ]; then
    print_failure "$violations"
    exit 1
fi
echo "line width check passed"
