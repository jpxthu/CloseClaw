#!/usr/bin/env bash
#
# test-audit.sh — 静态扫描 + 运行时监控测试合规
# 用法: scripts/test-audit.sh [--runtime] [--target <pattern>]
#
# --runtime    用 strace 包住 cargo test --lib，抓运行时文件写入和网络 syscall
# --target PAT 只扫描文件路径中包含 PAT 的 .rs 文件（静态）
#              runtime 模式下同时传递给 cargo test（如 --target permission）

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC_DIR="$PROJECT_ROOT/src"
TEST_DIR="$PROJECT_ROOT/tests"

RUNTIME=0
TARGET_FILTER=""

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
    -h|--help)
      echo "用法: $0 [--runtime] [--target <pattern>]"
      echo ""
      echo "选项:"
      echo "  --runtime       用 strace 包住 cargo test --lib，抓运行时违规"
      echo "  --target PAT    过滤扫描/测试范围"
      exit 0
      ;;
    *)
      echo "未知参数: $1" >&2
      exit 2
      ;;
  esac
done

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

  TRACE_LOG="/tmp/test-audit-trace-$$.log"

  # 构建 cargo test 命令
  CARGO_ARGS=(--lib)
  if [[ -n "$TARGET_FILTER" ]]; then
    CARGO_ARGS+=(-- "$TARGET_FILTER")
  fi

  echo "[test-audit] 运行时监控: strace cargo test ${CARGO_ARGS[*]}" >&2

  # 执行 strace 包裹 cargo test
  set +e
  strace -f -e trace=openat,creat,mkdir,rmdir,unlink,rename,socket,connect \
    -o "$TRACE_LOG" \
    cargo "${CARGO_ARGS[@]}" 2>/dev/null
  STRACE_EXIT=$?
  set -e

  # ── 后处理 trace log ──────────────────────────────────
  if [[ -f "$TRACE_LOG" ]]; then
    # 检查 1: openat(..., O_CREAT) 路径不在 /tmp/ 且不在 target/ → 告警
    while IFS= read -r line; do
      path=$(echo "$line" | sed -n 's/.*openat([^,]*, "\([^"]*\)".*/\1/p')
      if [[ -n "$path" ]] && [[ ! "$path" == /tmp/* ]] && [[ ! "$path" == */target/* ]]; then
        echo "[strace] ${path}: runtime 违规: 文件写入(非 temp 目录)"
        VIOLATIONS=$((VIOLATIONS + 1))
      fi
    done < <(grep -n "O_CREAT" "$TRACE_LOG" 2>/dev/null || true)

    # 检查 2: connect( 目标非 127.0.0.1 / ::1 → 告警
    local sed_inet4='s/.*connect([^,]*, {sa_family=AF_INET'\
'[^,]*, sin_port=htons(\([0-9]*\)), sin_addr=inet_addr'\
'("\([^"]*\)")).*/\2:\1/p'
    local sed_inet6='s/.*connect([^,]*, {sa_family=AF_INET6'\
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

    # 检查 3: mkdir/unlink/rmdir 路径不在 temp 目录 → 告警
    while IFS= read -r line; do
      path=$(echo "$line" | sed -n 's/.*\(mkdir\|unlink\|rmdir\)("\([^"]*\)").*/\2/p')
      if [[ -n "$path" ]] && [[ ! "$path" == /tmp/* ]] && [[ ! "$path" == */target/* ]]; then
        echo "[strace] ${path}: runtime 违规: 非 temp 目录操作"
        VIOLATIONS=$((VIOLATIONS + 1))
      fi
    done < <(grep -E "(mkdir|unlink|rmdir)\(" "$TRACE_LOG" 2>/dev/null || true)

    # 清理
    rm -f "$TRACE_LOG"
  fi

  # 检查 cargo test 本身是否失败
  if [[ $STRACE_EXIT -ne 0 ]]; then
    echo "[test-audit] 警告: cargo test 退出码 ${STRACE_EXIT}" >&2
  fi

  echo "[test-audit] 运行时监控完成" >&2
fi

# ══════════════════════════════════════════════════════════
# 静态扫描
# ══════════════════════════════════════════════════════════

# ── 扫描规则 ──────────────────────────────────────────────
# 用单独数组存储 label 和 pattern，避免分隔符冲突

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

# ── 执行扫描 ──────────────────────────────────────────────
for i in "${!LABELS[@]}"; do
  label="${LABELS[$i]}"
  pattern="${PATTERNS[$i]}"

  # 构建 find 命令参数
  FIND_ARGS=("$SRC_DIR" "$TEST_DIR" -name '*.rs' -type f)
  if [[ -n "$TARGET_FILTER" ]]; then
    FIND_ARGS+=(-path "*${TARGET_FILTER}*")
  fi

  # grep 扫描
  while IFS= read -r file; do
    while IFS= read -r match; do
      line="${match%%:*}"
      echo "${file}:${line}: ${label}"
      VIOLATIONS=$((VIOLATIONS + 1))
    done < <(grep -nE "$pattern" "$file" 2>/dev/null || true)
  done < <(find "${FIND_ARGS[@]}" 2>/dev/null)
done

# ── 结果 ──────────────────────────────────────────────────
if [[ $VIOLATIONS -gt 0 ]]; then
  echo "" >&2
  echo "[test-audit] 发现 ${VIOLATIONS} 处违规" >&2
  exit 1
else
  echo "[test-audit] 通过 — 未发现违规" >&2
  exit 0
fi
