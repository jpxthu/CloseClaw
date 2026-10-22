#!/usr/bin/env bash
#
# test-audit.sh — 静态扫描 + 运行时监控测试合规
# 用法: scripts/test-audit.sh [--runtime] [--target <pattern>] [--update-baseline]
#
# --runtime          用 strace 包住 cargo nextest run --lib，抓运行时文件写入和网络 syscall
# --target PAT       只扫描文件路径中包含 PAT 的 .rs 文件（静态）
#                    runtime 模式下同时传递给 nextest（如 --target permission）
# --update-baseline  以当前静态扫描结果全量重写 baseline 文件
#                    （scripts/test-audit-baseline.txt）；运行时违规不入 baseline；
#                    与 --target 互斥（baseline 必须来自全量扫描，避免过滤子集覆盖）

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BASELINE_FILE="$PROJECT_ROOT/scripts/test-audit-baseline.txt"
# 系统临时目录：$TMPDIR，未设置回退 /tmp（循环剥除全部尾斜杠归一化，供 runtime 白名单前缀与静态字面量检测使用）
TEMP_ROOT="${TMPDIR:-/tmp}"
while [[ "$TEMP_ROOT" == */ ]]; do
  TEMP_ROOT="${TEMP_ROOT%/}"
done

RUNTIME=0
TARGET_FILTER=""
UPDATE_BASELINE=0

# ── 参数解析 ──────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --runtime)
      RUNTIME=1
      shift
      ;;
    --target)
      if [[ -z "${2:-}" ]]; then
        echo "错误: --target 需要一个参数" >&2
        exit 2
      fi
      TARGET_FILTER="$2"
      shift 2
      ;;
    --update-baseline)
      UPDATE_BASELINE=1
      shift
      ;;
    -h|--help)
      echo "用法: $0 [--runtime] [--target <pattern>] [--update-baseline]"
      echo ""
      echo "选项:"
      echo "  --runtime           用 strace 包住 cargo nextest run --lib，抓运行时违规"
      echo "  --target PAT        过滤扫描/测试范围"
      echo "  --update-baseline   以当前静态扫描结果全量重写 baseline 文件（与 --target 互斥）"
      exit 0
      ;;
    *)
      echo "未知参数: $1" >&2
      exit 2
      ;;
  esac
done

if [[ $UPDATE_BASELINE -eq 1 && -n "$TARGET_FILTER" ]]; then
  echo "错误: --update-baseline 与 --target 互斥（baseline 必须来自全量扫描）" >&2
  exit 2
fi

VIOLATIONS=0

# ══════════════════════════════════════════════════════════
# 运行时监控（--runtime）
# ══════════════════════════════════════════════════════════
if [[ $RUNTIME -eq 1 ]]; then
  # 检查 strace 可用性
  if ! command -v strace &>/dev/null; then
    echo "[test-audit] 错误: 未找到 strace，无法执行运行时监控" >&2
    exit 2
  fi

  TRACE_LOG="$(mktemp)"

  # 构建 nextest 命令
  CARGO_ARGS=(nextest run --lib)
  if [[ -n "$TARGET_FILTER" ]]; then
    CARGO_ARGS+=(-- "$TARGET_FILTER")
  fi

  echo "[test-audit] 运行时监控: strace cargo ${CARGO_ARGS[*]}" >&2

  # 执行 strace 包裹 cargo nextest
  set +e
  strace -f -e trace=openat,creat,mkdir,rmdir,unlink,rename,socket,connect \
    -o "$TRACE_LOG" \
    cargo "${CARGO_ARGS[@]}" 2>/dev/null
  STRACE_EXIT=$?
  set -e

  # ── 后处理 trace log ──────────────────────────────────
  if [[ -f "$TRACE_LOG" ]]; then
    # 检查 1: openat(..., O_CREAT) 路径不在 temp 目录(TEMP_ROOT) 且不在 target/ → 告警
    while IFS= read -r line; do
      path=$(echo "$line" | sed -n 's/.*openat([^,]*, "\([^"]*\)".*/\1/p')
      if [[ -n "$path" ]] && [[ ! "$path" == "$TEMP_ROOT"/* ]] && [[ ! "$path" == */target/* ]]; then
        echo "[strace] ${path}: runtime 违规: 文件写入(非 temp 目录)"
        VIOLATIONS=$((VIOLATIONS + 1))
      fi
    done < <(grep -n "O_CREAT" "$TRACE_LOG" 2>/dev/null || true)

    # 检查 2: connect( 目标非 127.0.0.1 / ::1 → 告警
    sed_inet4='s/.*connect([^,]*, {sa_family=AF_INET'\
'[^,]*, sin_port=htons(\([0-9]*\)), sin_addr=inet_addr'\
'("\([^"]*\)")).*/\2:\1/p'
    sed_inet6='s/.*connect([^,]*, {sa_family=AF_INET6'\
'[^}]*, inet_pton(AF_INET6, "\([^"]*\)").*/\1/p'
    while IFS= read -r line; do
      # 提取 connect 的 sockaddr 地址
      addr=$(echo "$line" | sed -n "$sed_inet4")
      if [[ -z "$addr" ]]; then
        # 尝试 IPv6 格式
        addr=$(echo "$line" | sed -n "$sed_inet6")
      fi
      if [[ -n "$addr" ]] && [[ "$addr" != "127.0.0.1:"* ]] && [[ "$addr" != "::1" ]]; then
        echo "[strace] connect → ${addr}: runtime 违规: 外部网络访问"
        VIOLATIONS=$((VIOLATIONS + 1))
      fi
    done < <(grep "connect(" "$TRACE_LOG" 2>/dev/null || true)

    # 检查 3: mkdir/unlink/rmdir 路径不在 temp 目录(TEMP_ROOT) → 告警
    while IFS= read -r line; do
      path=$(echo "$line" | sed -n 's/.*\(mkdir\|unlink\|rmdir\)("\([^"]*\)").*/\2/p')
      if [[ -n "$path" ]] && [[ ! "$path" == "$TEMP_ROOT"/* ]] && [[ ! "$path" == */target/* ]]; then
        echo "[strace] ${path}: runtime 违规: 非 temp 目录操作"
        VIOLATIONS=$((VIOLATIONS + 1))
      fi
    done < <(grep -E "(mkdir|unlink|rmdir)\(" "$TRACE_LOG" 2>/dev/null || true)

    # 清理
    rm -f "$TRACE_LOG"
  fi

  # 检查 nextest 本身是否失败
  if [[ $STRACE_EXIT -ne 0 ]]; then
    echo "[test-audit] 警告: cargo nextest run 退出码 ${STRACE_EXIT}" >&2
  fi

  echo "[test-audit] 运行时监控完成" >&2
fi

# ══════════════════════════════════════════════════════════
# 静态扫描
# ══════════════════════════════════════════════════════════

# ── 扫描规则 ──────────────────────────────────────────────
# 用单独数组存储 label、pattern 和 grep 模式（E=扩展正则 / F=字面量），避免分隔符冲突

LABELS=(
  "环境变量泄漏"
  "手动目录管理"
  "硬编码路径"
  "网络socket(review)"
)

PATTERNS=(
  'env::set_var|env::remove_var'
  'create_dir_all|remove_dir_all'
  '"/tmp'
  'TcpStream|TcpListener|UdpSocket'
)

MODES=(
  E
  E
  E
  E
)

# ── TEMP_ROOT 字面量补充规则 ──────────────────────────────
# 当前 TEMP_ROOT 的硬编码字面量同样计为「硬编码路径」违规；
# TEMP_ROOT 以 /tmp 开头时已由 '"/tmp'（PATTERNS[2]，-E 子串匹配）覆盖，跳过避免双计数；
# 用 grep -F 精确匹配，TEMP_ROOT 含正则元字符时不受影响
if [[ "$TEMP_ROOT" != /tmp* ]]; then
  LABELS+=("${LABELS[2]}")
  PATTERNS+=("\"$TEMP_ROOT")
  MODES+=(F)
fi

# ── 扫描目录集合 ──────────────────────────────────────────
# 顶层 src/、tests/ + crates/*/src（glob 展开）；文件名匹配模式为既有设计，
# 扩围只扩展扫描根目录（find 模式不变）
SCAN_DIRS=("$PROJECT_ROOT/src" "$PROJECT_ROOT/tests")
for dir in "$PROJECT_ROOT"/crates/*/src; do
  if [[ -d "$dir" ]]; then
    SCAN_DIRS+=("$dir")
  fi
done

# ── 执行扫描（收集条目，输出延后到 baseline 比对）─────────
# 构建 find 命令参数（仅依赖 TARGET_FILTER，循环不变量，构建一次）
FIND_ARGS=("${SCAN_DIRS[@]}" -type f \( -name '*_tests.rs' -o -path '*/tests/*' -o -name 'tests.rs' \))
if [[ -n "$TARGET_FILTER" ]]; then
  FIND_ARGS+=(-path "*${TARGET_FILTER}*")
fi

ENTRIES=()
for i in "${!LABELS[@]}"; do
  label="${LABELS[$i]}"
  pattern="${PATTERNS[$i]}"
  mode="${MODES[$i]}"

  # grep 扫描；条目键 = 相对路径:行号: 规则label（与 baseline 条目一致）
  while IFS= read -r file; do
    rel_path="${file#"$PROJECT_ROOT"/}"
    while IFS= read -r match; do
      line="${match%%:*}"
      ENTRIES+=("${rel_path}:${line}: ${label}")
    done < <(grep -n"$mode" -- "$pattern" "$file" 2>/dev/null || true)
  done < <(find "${FIND_ARGS[@]}" 2>/dev/null)
done

# 去重排序（C locale 字节序，保证与 baseline 比对及重写结果确定性）
ACTUAL=()
if [[ ${#ENTRIES[@]} -gt 0 ]]; then
  mapfile -t ACTUAL < <(printf '%s\n' "${ENTRIES[@]}" | LC_ALL=C sort -u)
fi

# ── --update-baseline：全量重写 baseline 文件 ─────────────
if [[ $UPDATE_BASELINE -eq 1 ]]; then
  {
    echo "# test-audit baseline — 存量豁免清单（自动生成）"
    echo "# 键格式: 相对路径:行号: 规则label（与脚本违规输出一致）"
    echo "# 语义: 命中条目豁免；清单外新增违规 fail；已消除条目提示可收窄"
    echo "# 重写: bash scripts/test-audit.sh --update-baseline"
    if [[ ${#ACTUAL[@]} -gt 0 ]]; then
      printf '%s\n' "${ACTUAL[@]}"
    fi
  } > "$BASELINE_FILE"
  echo "[test-audit] baseline 已全量重写: scripts/test-audit-baseline.txt（${#ACTUAL[@]} 条）" >&2
  exit 0
fi

# ── baseline 三态比对 ─────────────────────────────────────
# 新增（baseline 外）→ 列出并 fail；存量（baseline 内）→ 豁免；
# 已消除（baseline 有、实际无）→ 打「可收窄」警告（不 fail）。
# --target 过滤模式下跳过「已消除」判定（过滤子集无法证明全量消除）。
BASELINE=()
if [[ -f "$BASELINE_FILE" ]]; then
  mapfile -t BASELINE < <(
    sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' "$BASELINE_FILE" \
      | grep -vE '^(#|$)' | LC_ALL=C sort -u
  )
fi

declare -A BASELINE_SET=()
for entry in "${BASELINE[@]}"; do
  BASELINE_SET["$entry"]=1
done

NEW_VIOLATIONS=()
EXEMPT_COUNT=0
for entry in "${ACTUAL[@]}"; do
  if [[ -z "${BASELINE_SET[$entry]+x}" ]]; then
    NEW_VIOLATIONS+=("$entry")
  else
    EXEMPT_COUNT=$((EXEMPT_COUNT + 1))
  fi
done

ELIMINATED=()
if [[ -f "$BASELINE_FILE" && -z "$TARGET_FILTER" ]]; then
  declare -A ACTUAL_SET=()
  for entry in "${ACTUAL[@]}"; do
    ACTUAL_SET["$entry"]=1
  done
  for entry in "${BASELINE[@]}"; do
    if [[ -z "${ACTUAL_SET[$entry]+x}" ]]; then
      ELIMINATED+=("$entry")
    fi
  done
fi

# ── 结果 ──────────────────────────────────────────────────
NEW_COUNT=${#NEW_VIOLATIONS[@]}
if [[ $NEW_COUNT -gt 0 ]]; then
  printf '%s\n' "${NEW_VIOLATIONS[@]}"
fi

if [[ ${#ELIMINATED[@]} -gt 0 ]]; then
  echo "[test-audit] ${#ELIMINATED[@]} 条 baseline 条目已消除，可收窄（bash scripts/test-audit.sh --update-baseline 重写）:" >&2
  printf '  %s\n' "${ELIMINATED[@]}" >&2
fi

if [[ $NEW_COUNT -gt 0 || $VIOLATIONS -gt 0 ]]; then
  echo "[test-audit] 失败 — baseline 外新增违规 ${NEW_COUNT} 处，运行时违规 ${VIOLATIONS} 处，baseline 豁免 ${EXEMPT_COUNT} 处" >&2
  exit 1
fi

echo "[test-audit] 通过 — 静态命中 ${#ACTUAL[@]} 处全部在 baseline 内（豁免 ${EXEMPT_COUNT} 处），已消除 ${#ELIMINATED[@]} 条（可 --update-baseline 收窄），运行时违规 0" >&2
exit 0
