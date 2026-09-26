#!/usr/bin/env bash
#
# check-env-var.sh — std::env::set_var / remove_var 禁令检查（CI 与 pre-commit 共享的单一实现）
#
# 用途: 禁令判定与错误文案的单一来源。ci.yml「Check env var prohibition」与 .githooks/pre-commit
#       均调用本脚本，避免重复实现与文案分叉。
# 用法: bash scripts/check-env-var.sh [all|staged]
#         all    扫描全部 tracked *.rs（git ls-files，天然排除 target/ 与未跟踪文件），CI 用（默认）
#         staged 只判定 staged 新增行（*.rs，ACMR），pre-commit 用
# 判定: 行级文本匹配 set_var / remove_var（可能命中注释或同名标识符，非必然为调用）；
#       行内含 load_env_file 标记即豁免（行级文本标记豁免）。
# 退出码: 0=通过, 1=命中禁令, 2=用法错误 / 非 git 仓库
#
# 细则: docs/developer/STANDARDS.md §7（并行安全）、CONTRIBUTING.md「测试 > 安全红线」
# 唯一豁免点: crates/daemon/src/mod.rs 的 load_env_file()
set -euo pipefail

# 判定程序（单一来源）：all / staged 两种模式共用 is_hit() 的匹配与豁免逻辑
AWK_PROG='
function is_hit(line) {
    if (line ~ /load_env_file/) return 0
    if (line ~ /(^|[^A-Za-z0-9_])set_var([^A-Za-z0-9_]|$)/) return 1
    if (line ~ /(^|[^A-Za-z0-9_])remove_var([^A-Za-z0-9_]|$)/) return 1
    return 0
}
mode == "all" {
    if (is_hit($0)) printf "%s:%d:%s\n", FILENAME, FNR, $0
    next
}
mode == "staged" {
    if (substr($0, 1, 3) == "+++") next
    if (substr($0, 1, 2) == "@@") {
        if (match($0, /[+][0-9]+(,[0-9]+)?/)) {
            split(substr($0, RSTART + 1, RLENGTH - 1), a, ",")
            newln = a[1] + 0
        }
        next
    }
    if (substr($0, 1, 1) == "+") {
        content = substr($0, 2)
        if (is_hit(content)) printf "%s:%d:%s\n", file, newln, content
        newln++
    }
}
'

# all 模式：扫描 git ls-files 列出的全部 tracked *.rs
collect_all() {
    local files=()
    local f
    while IFS= read -r -d '' f; do
        if [ -f "$f" ]; then
            files+=("$f")
        fi
    done < <(git ls-files -z '*.rs')
    if [ "${#files[@]}" -eq 0 ]; then
        return 0
    fi
    awk -v mode=all "$AWK_PROG" "${files[@]}"
}

# staged 模式：只判定 staged 新增行（行为等价旧 hook：ACMR 的 + 行 × *.rs）
collect_staged() {
    local staged_rs
    local f
    staged_rs=$(git diff --cached --name-only --diff-filter=ACMR -- '*.rs')
    if [ -z "$staged_rs" ]; then
        return 0
    fi
    while IFS= read -r f; do
        if [ -z "$f" ]; then
            continue
        fi
        git diff --cached --diff-filter=ACMR --no-color --unified=0 -- "$f" \
            | awk -v mode=staged -v file="$f" "$AWK_PROG"
    done <<< "$staged_rs"
}

# 统一错误文案（唯一来源，禁令理由 + 正确做法 + 正确引用 + 真实豁免机制）
print_failure() {
    echo "ERROR: 检测到 std::env::set_var / remove_var 禁令命中（行级文本匹配，可能命中注释或同名标识符，非必然为调用）："
    echo "$1"
    echo ""
    echo "环境变量修改在多线程和并行测试中会导致数据竞争，全代码库禁止使用。"
    echo "正确做法："
    echo "  - 配置值通过参数/config struct 传递，不写入全局 env"
    echo "  - 测试中用依赖注入或临时文件路径代替 set_var"
    echo "  - 只读取环境变量请用 std::env::var（安全）"
    echo ""
    echo "唯一例外：crates/daemon/src/mod.rs 的 load_env_file()（启动阶段加载 .env 文件），"
    echo "按行级文本标记豁免：命中行内含 load_env_file 标记即豁免。"
    echo ""
    echo "详见 docs/developer/STANDARDS.md §7（并行安全）与 CONTRIBUTING.md「测试 > 安全红线」。"
}

usage() {
    echo "usage: $0 [all|staged]" >&2
}

if [ "$#" -gt 1 ]; then
    usage
    exit 2
fi
mode="${1:-all}"
case "$mode" in
    all | staged) ;;
    *)
        usage
        exit 2
        ;;
esac

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "ERROR: $0 必须在 git 仓库内运行（无法执行禁令检查）" >&2
    exit 2
fi

hits="$(collect_"$mode")"
if [ -n "$hits" ]; then
    print_failure "$hits"
    exit 1
fi
echo "env var check passed"
