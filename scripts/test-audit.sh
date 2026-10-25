#!/usr/bin/env bash
#
# test-audit.sh — 静态扫描 + 运行时监控测试合规
# 用法: scripts/test-audit.sh [--runtime] [--target <pattern>] [--update-baseline]
#                            [--self-test]
#
# 静态扫描范围：独立测试文件全文件（*_tests.rs / tests.rs / tests 目录），
# 以及扩围通道——生产文件内联 #[cfg(test)] 门控区域 + cfg(test) 挂载的
# test-only 文件。helper 收敛点本体（平铺 helper 文件 + 目录化 helper 子模块）
# 经三通道一致的豁免判据排除：文件名含 test_helpers / test_support 词干，
# 或路径含 /test_helpers/、/test_support/ 段（见 is_helper_exempt）
#
# 扩围通道已知局限（best-effort 行级解析，不追全量 Rust 语法）：
#   - cfg 属性跨行书写不识别（如 `#[cfg(all(
#     test, ...))]` 拆行形态）
#   - `#[path]` 属性位于 cfg(test) 之前的逆序形态不识别
#   - 声明与 `{` 跨行分离（如 `mod tests` 换行 `{`）不进区域
#   - 块注释 /* */ 不剥离，其中花括号仍可能扰动区域边界
#   - 字节 raw string（br#"…"#）不识别为 raw string 起点（退化为普通字符串
#     行级剥离，跨行仍可能扰动边界）
# （普通字符串字面量在深度计数前剥离，含 \" 与 \\ 转义；raw string
# r#"…"#（r 前缀 + 任意井号数）已跨行剥离，花括号不扰动区域边界）
#
# --runtime          用 strace 包住 cargo nextest run --lib，抓运行时文件写入和网络 syscall
# --target PAT       只扫描文件路径中包含 PAT 的 .rs 文件（静态）
#                    runtime 模式下同时传递给 nextest（如 --target permission）
# --update-baseline  以当前静态扫描结果全量重写 baseline 文件
#                    （scripts/test-audit-baseline.txt）；运行时违规不入 baseline；
#                    与 --target、--runtime 互斥（baseline 必须来自全量扫描，
#                    且不得吞掉 runtime 违规报告）
# --self-test        扫描器自测：对 mktemp 内嵌最小 fixture 树运行与正常模式
#                    完全相同的扫描实现，断言命中/不命中规格矩阵（内联区域
#                    命中 / 生产区不命中 / not(test) 不命中 / helper 豁免
#                    （平铺命名 + 目录化子模块 + 文件内内联区域）/
#                    挂载解析含 #[path] 与非 mod.rs 声明者 / 字面量剥离
#                    （普通 + 多行 raw string）/ 原谓词文件无双计）；离线、不读仓库与 baseline；
#                    与 --runtime/--target/--update-baseline 互斥

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
SELF_TEST=0

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
    --self-test)
      SELF_TEST=1
      shift
      ;;
    -h|--help)
      echo "用法: $0 [--runtime] [--target <pattern>] [--update-baseline] [--self-test]"
      echo ""
      echo "选项:"
      echo "  --runtime           用 strace 包住 cargo nextest run --lib，抓运行时违规"
      echo "  --target PAT        过滤扫描/测试范围"
      echo "  --update-baseline   以当前静态扫描结果全量重写 baseline 文件（与 --target、--runtime 互斥）"
      echo "  --self-test         内嵌最小 fixture 自测扫描器，断言命中/不命中规格矩阵（离线）"
      exit 0
      ;;
    *)
      echo "未知参数: $1" >&2
      exit 2
      ;;
  esac
done

if [[ $SELF_TEST -eq 1 ]]; then
  if [[ $RUNTIME -eq 1 || -n "$TARGET_FILTER" || $UPDATE_BASELINE -eq 1 ]]; then
    echo "错误: --self-test 与 --runtime/--target/--update-baseline 互斥" >&2
    exit 2
  fi
fi

if [[ $UPDATE_BASELINE -eq 1 && -n "$TARGET_FILTER" ]]; then
  echo "错误: --update-baseline 与 --target 互斥（baseline 必须来自全量扫描）" >&2
  exit 2
fi

if [[ $UPDATE_BASELINE -eq 1 && $RUNTIME -eq 1 ]]; then
  echo "错误: --update-baseline 与 --runtime 互斥（防 runtime 违规被 baseline 重写吞掉）" >&2
  exit 2
fi

# ── 临时文件统一清理（EXIT 兜底，覆盖错误退出路径）────────
TRACE_LOG=""
EVENTS_FILE=""
MTARGETS_FILE=""
FILTERED_EVENTS_FILE=""
FIXTURE_DIR=""
cleanup_temp_files() {
  local f
  for f in "$TRACE_LOG" "$EVENTS_FILE" "$MTARGETS_FILE" "$FILTERED_EVENTS_FILE"; do
    if [[ -n "$f" ]]; then
      rm -f "$f"
    fi
  done
  if [[ -n "$FIXTURE_DIR" ]]; then
    rm -rf "$FIXTURE_DIR"
  fi
}
trap cleanup_temp_files EXIT

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

  TRACE_LOG="$(mktemp "${TMPDIR:-/tmp}/test-audit-trace-XXXXXX")"

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

# ── 静态扫描执行 ──────────────────────────────────────────
# 扫描实现整体函数化（collect_static_entries <扫描根目录>，结果写入全局
# ENTRIES）：正常模式对 $PROJECT_ROOT 调用；--self-test 对 mktemp 内嵌
# fixture 树调用同一实现，保证「测的就是真实扫描逻辑」（函数定义在
# REGION_SCANNER_AWK 之后，调用点在 run_self_test / 正常模式处）

# ══════════════════════════════════════════════════════════
# 扩围通道：cfg(test) 门控区域（内联）+ cfg(test) 挂载 test-only 文件
# ══════════════════════════════════════════════════════════
# 原通道只全文件扫独立测试文件（*_tests.rs / tests.rs / tests 目录），存在盲区：
#   A. 生产文件内联 #[cfg(test)] 门控区域（mod/fn/impl 块，含 all(test,...)/
#      any(test,...) 复合形态；排除 not(test) 与 feature = "test-*" 命名误配）
#      内的规则命中
#   B. #[cfg(test)] [#[path = "..."]] mod NAME; 挂载、命名特殊的 test-only 文件
# 双计防护：两通道仅处理「原 find 谓词不匹配且非 helper 豁免的 .rs 文件」
# （与原通道互补，豁免口径一致）；挂载目标再从内联通道剔除（挂载文件走全
# 文件扫）。helper 收敛点本体不进扫描视野：原通道文件收集、补集通道文件
# 收集、挂载目标解析三处统一引用 is_helper_exempt（basename ∪ 路径段并集，
# 见函数注释），「helper 本体（平铺 + 目录化子模块，含文件内内联 cfg(test)
# 区域）不扫」为机制保证，非「helper 文件恰好无 cfg(test) 区域」的侥幸等价。

# ── cfg(test) 区域/挂载扫描器（awk）──────────────────────
# 输出事件（TSV）：
#   H\t相对路径\t行号\t行内容 = cfg(test) 门控区域内的行（花括号深度定位边界，
#                             深度计数前剥离 // 行注释、普通字符串字面量
#                             （best-effort，含 \" \\ 转义）与 raw string
#                             （r#"…"# 含任意井号数，跨行状态机；块注释
#                             为已知局限，见脚本头注释）；逐文件复位状态，可分批）
#   M\t相对路径\tpath属性\tmod名 = cfg(test) 门控的 mod 挂载声明（path 属性可空）
REGION_SCANNER_AWK="$(cat <<'AWK_PROGRAM'
function trim_ws(x) {
  gsub(/^[[:space:]]+/, "", x)
  gsub(/[[:space:]]+$/, "", x)
  return x
}
# 正则拆行为动态字符串拼接（等价改写，单行 ≤100 字符；\ 为 awk 续行符）
function is_item_decl(s) {
  return s ~ ("^([[:space:]]*)?(pub([[:space:]]*\\([a-z]+\\))?([[:space:]]+))?" \
              "(unsafe[[:space:]]+)?(mod|fn|impl)[[:space:]]")
}
function is_mount_decl(s) {
  return is_item_decl(s) && s ~ /(^|[[:space:]])mod[[:space:]]+/ && s ~ /;[[:space:]]*$/
}
function pred_has_test(s) {
  if (s !~ /^#\[[[:space:]]*cfg[[:space:]]*\(/) return 0
  if (s ~ /not[[:space:]]*\([[:space:]]*(all|any)?[[:space:]]*\(?[[:space:]]*test/) return 0
  return s ~ /(^|[^a-zA-Z0-9_-])test([^a-zA-Z0-9_-]|$)/
}
# 字符串字面量剥离（供深度计数与 `{` 存在性判断使用）：
# 普通字符串含 \" 与 \\ 转义，行内未闭合视为延伸到行尾；raw string（`r` 前缀
# + ≥0 个 #，如 r#"…"# / r##"…"##）经 rs_open/rs_hash 全局状态跨行剥离：
# 开启后本行及后续行至匹配井号数的 `"#` 闭合前的所有字符（含花括号）不进入
# 输出，不参与深度计数（状态逐文件复位）；消费段以空格占位避免词元粘连；
# 块注释 /* */ 为已知局限（见脚本头注释），不在本函数处理范围
function strip_strings(x,   out, i, n, ch, j, m, ok) {
  out = ""
  i = 1
  n = length(x)
  while (i <= n) {
    if (rs_open) {
      # raw string 内：仅识别闭合 `"#…#`（井号数 = 开启时 rs_hash）
      j = i
      while (j <= n && substr(x, j, 1) != "\"") j++
      if (j > n) {
        out = out " "
        i = n + 1
        continue
      }
      ok = 1
      m = 0
      while (m < rs_hash) {
        if (substr(x, j + 1 + m, 1) != "#") {
          ok = 0
          break
        }
        m++
      }
      if (ok) {
        i = j + 1 + rs_hash
        rs_open = 0
      } else {
        i = j + 1
      }
      continue
    }
    ch = substr(x, i, 1)
    if (ch == "\"") {
      out = out " "
      i++
      while (i <= n) {
        ch = substr(x, i, 1)
        if (ch == "\\") {
          i += 2
          continue
        }
        i++
        if (ch == "\"") {
          break
        }
      }
    } else if (ch == "r" && (i == 1 || substr(x, i - 1, 1) !~ /[A-Za-z0-9_]/)) {
      # raw string 起始：r + ≥0 个 # + "（r#type 等 raw 标识符无引号，不误配）
      j = i + 1
      while (substr(x, j, 1) == "#") j++
      if (substr(x, j, 1) == "\"") {
        rs_open = 1
        rs_hash = j - i - 1
        out = out " "
        i = j + 1
      } else {
        out = out ch
        i++
      }
    } else {
      out = out ch
      i++
    }
  }
  return out
}
BEGIN {
  root_len = length(root)
  depth = 0
  nb = 0
  pending = 0
  mount_path = ""
  rs_open = 0
  rs_hash = 0
}
FNR == 1 {
  depth = 0
  nb = 0
  pending = 0
  mount_path = ""
  rs_open = 0
  rs_hash = 0
  rel = substr(FILENAME, root_len + 2)
}
{
  raw = $0
  sub(/\/\/.*/, "", raw)
  s = trim_ws(raw)

  in_region = (nb > 0 && depth > bases[nb]) ? 1 : 0

  brace_code = strip_strings(raw)
  has_ob = (brace_code ~ /{/) ? 1 : 0

  if (pending) {
    # 行注释已在上方剥离，此处 s 不可能以 // 开头（原死条件已删）
    if (s != "") {
      pending = 0
      if (s ~ /^#\[/) {
        if (s ~ /^#\[[[:space:]]*path[[:space:]]*=/) {
          mount_path = s
          sub(/^#\[[[:space:]]*path[[:space:]]*=[[:space:]]*"/, "", mount_path)
          sub(/"[[:space:]]*\].*$/, "", mount_path)
        }
        pending = 1
      } else if (is_item_decl(s)) {
        if (is_mount_decl(s)) {
          name = s
          sub(/^.*(^|[[:space:]])mod[[:space:]]+/, "", name)
          sub(/;.*$/, "", name)
          printf "M\t%s\t%s\t%s\n", rel, mount_path, name
        } else if (has_ob) {
          bases[++nb] = depth
        }
        mount_path = ""
      } else {
        mount_path = ""
      }
    }
  } else if (pred_has_test(s)) {
    rest = s
    # 循环剥离前导 #[...] 属性组（单行多属性形态），沿途捕获 #[path] 值
    while (rest ~ /^#[^]]*\][[:space:]]*/) {
      if (rest ~ /^#\[[[:space:]]*path[[:space:]]*=/) {
        p = rest
        sub(/^#[^]]*=[[:space:]]*"/, "", p)
        sub(/".*$/, "", p)
        mount_path = p
      }
      sub(/^#[^]]*\][[:space:]]*/, "", rest)
    }
    if (rest == "") {
      pending = 1
    } else if (is_item_decl(rest)) {
      if (is_mount_decl(rest)) {
        name = rest
        sub(/^.*(^|[[:space:]])mod[[:space:]]+/, "", name)
        sub(/;.*$/, "", name)
        printf "M\t%s\t%s\t%s\n", rel, mount_path, name
      } else if (has_ob) {
        bases[++nb] = depth
      }
      mount_path = ""
    } else {
      mount_path = ""
    }
  }

  if (in_region) printf "H\t%s\t%d\t%s\n", rel, FNR, $0

  o = gsub(/{/, "{", brace_code)
  c = gsub(/}/, "}", brace_code)
  depth += o - c
  while (nb > 0 && depth <= bases[nb]) nb--
}
AWK_PROGRAM
)"

# ── helper 豁免判据（一处定义，三通道引用）────────────────
# basename ∪ 路径段并集（入参为相对 scan_root 的路径）：
#   - 文件名含 test_helpers / test_support 词干 → 平铺 helper 本体
#     （如 crates/common/src/test_helpers.rs，路径无段且为 cfg(test) 挂载目标）
#   - 路径含 /test_helpers/、/test_support/ 段 → 目录化 helper 子模块
#     （如 crates/daemon/src/test_helpers/*.rs）与 test_support 整目录
# 原通道文件收集、补集通道文件收集、挂载目标解析三处统一引用，
# 使「helper 收敛点本体不进扫描视野」成为机制保证而非侥幸等价
is_helper_exempt() {
  local rel="$1" base
  base="${rel##*/}"
  case "$base" in
    *test_helpers* | *test_support*) return 0 ;;
  esac
  case "$rel" in
    */test_helpers/* | */test_support/*) return 0 ;;
  esac
  return 1
}

collect_static_entries() {
  local scan_root="$1"
  ENTRIES=()

  # ── 扫描目录集合 ────────────────────────────────────────
  # 顶层 src/、tests/ + crates/*/src（glob 展开）；文件名匹配模式为既有设计；
  # 扩围不修改原 find 谓词，以「互补文件集 + 新通道」方式补充扫描（见下）
  local scan_dirs=("$scan_root/src" "$scan_root/tests")
  local sdir
  for sdir in "$scan_root"/crates/*/src; do
    if [[ -d "$sdir" ]]; then
      scan_dirs+=("$sdir")
    fi
  done

  # ── 原通道：独立测试文件全文件扫 ────────────────────────
  # 构建 find 命令参数（仅依赖 TARGET_FILTER，循环不变量，构建一次）
  local find_args=("${scan_dirs[@]}" -type f \(
    -name '*_tests.rs' -o -path '*/tests/*' -o -name 'tests.rs' \))
  if [[ -n "$TARGET_FILTER" ]]; then
    find_args+=(-path "*${TARGET_FILTER}*")
  fi

  # 文件清单只执行一次 find（规则循环不变量），逐规则复用；
  # helper 豁免在本通道同样生效（原通道可命中的 *_tests.rs 形态 helper
  # 测试文件按同一判据剔除，避免两通道豁免口径不一致）
  local rs_files=()
  mapfile -t rs_files < <(find "${find_args[@]}" 2>/dev/null)
  local scanned_rs=()
  for file in "${rs_files[@]}"; do
    if is_helper_exempt "${file#"$scan_root"/}"; then
      continue
    fi
    scanned_rs+=("$file")
  done
  rs_files=("${scanned_rs[@]}")

  local i label pattern mode file rel_path match line
  for i in "${!LABELS[@]}"; do
    label="${LABELS[$i]}"
    pattern="${PATTERNS[$i]}"
    mode="${MODES[$i]}"

    # grep 扫描；条目键 = 相对路径:行号: 规则label（与 baseline 条目一致）
    for file in "${rs_files[@]}"; do
      rel_path="${file#"$scan_root"/}"
      while IFS= read -r match; do
        line="${match%%:*}"
        ENTRIES+=("${rel_path}:${line}: ${label}")
      done < <(grep -n"$mode" -- "$pattern" "$file" 2>/dev/null || true)
    done
  done

  # ── 扩围文件集合：原 find 谓词的精确补集 ────────────────
  local all_find_args=("${scan_dirs[@]}" -type f -name '*.rs' \
    ! \( -name '*_tests.rs' -o -path '*/tests/*' -o -name 'tests.rs' \))
  if [[ -n "$TARGET_FILTER" ]]; then
    all_find_args+=(-path "*${TARGET_FILTER}*")
  fi

  EVENTS_FILE="$(mktemp "${TMPDIR:-/tmp}/test-audit-events-XXXXXX")"
  MTARGETS_FILE="$(mktemp "${TMPDIR:-/tmp}/test-audit-mtargets-XXXXXX")"
  FILTERED_EVENTS_FILE="$(mktemp "${TMPDIR:-/tmp}/test-audit-region-XXXXXX")"

  # 单次扫描同时产出内联区域事件与挂载声明（xargs 分批不影响逐文件状态复位）；
  # find 经 process substitution 隔离退出码（扫描目录可缺省，与原通道 rs_files 一致）
  local expansion_files=()
  mapfile -t expansion_files < <(find "${all_find_args[@]}" 2>/dev/null)
  # helper 豁免（路径段级）：目录化 helper 子模块与 helper 本体不经内联/
  # 挂载通道进扫描视野——否则「helper 不扫」仍依赖其恰好无 cfg(test) 区域
  local -a scanned_expansion=()
  for file in "${expansion_files[@]}"; do
    if is_helper_exempt "${file#"$scan_root"/}"; then
      continue
    fi
    scanned_expansion+=("$file")
  done
  if [[ ${#scanned_expansion[@]} -gt 0 ]]; then
    printf '%s\n' "${scanned_expansion[@]}" \
      | xargs -r -d '\n' awk -v root="$scan_root" "$REGION_SCANNER_AWK" > "$EVENTS_FILE"
  else
    : > "$EVENTS_FILE"
  fi

  # ── 挂载目标解析与过滤 ──────────────────────────────────
  local -A mount_target_seen=()
  local mline drel rest dattr dname ddir target declarer stem cand rel_target base
  local -a mount_candidates=()
  while IFS= read -r mline; do
    if [[ -z "$mline" ]]; then continue; fi
    drel="${mline%%$'\t'*}"
    rest="${mline#*$'\t'}"
    dattr="${rest%%$'\t'*}"
    dname="${rest##*$'\t'}"
    ddir="$(dirname "$scan_root/$drel")"
    if [[ -n "$dattr" ]]; then
      target="$ddir/$dattr"
    else
      # 无 #[path] 时按 Rust 2018 mod 解析规则：mod.rs / lib.rs / main.rs 声明者
      # → 兄弟位置候选；其余声明者（如 foo.rs）→ 子目录候选 $ddir/<stem>/<name>
      declarer="$(basename "$drel")"
      if [[ "$declarer" == mod.rs || "$declarer" == lib.rs || "$declarer" == main.rs ]]; then
        mount_candidates=("$ddir/$dname.rs" "$ddir/$dname/mod.rs")
      else
        stem="${declarer%.rs}"
        mount_candidates=("$ddir/$stem/$dname.rs" "$ddir/$stem/$dname/mod.rs")
      fi
      target=""
      for cand in "${mount_candidates[@]}"; do
        if [[ -f "$cand" ]]; then
          target="$cand"
          break
        fi
      done
      if [[ -z "$target" ]]; then
        continue
      fi
    fi
    if [[ ! -f "$target" ]]; then continue; fi
    target="$(realpath -m "$target")"
    if [[ "$target" != "$scan_root/"* ]]; then continue; fi
    rel_target="${target#"$scan_root"/}"
    # 原通道已覆盖（全文件扫）→ 剔除防双计
    base="$(basename "$rel_target")"
    if [[ "$base" == *_tests.rs || "$base" == tests.rs ]]; then continue; fi
    if [[ "$rel_target" == */tests/* ]]; then continue; fi
    # helper 收敛点本体不纳入（basename ∪ 路径段并集，判据见 is_helper_exempt；
    # $rel_target 含目录段，目录化 helper 本体在此按路径段豁免）
    if is_helper_exempt "$rel_target"; then continue; fi
    # --target 过滤语义与主通道一致
    if [[ -n "$TARGET_FILTER" && "$rel_target" != *"$TARGET_FILTER"* ]]; then continue; fi
    # 异常自指防护
    if [[ "$rel_target" == "$drel" ]]; then continue; fi
    if [[ -z "${mount_target_seen[$rel_target]:-}" ]]; then
      mount_target_seen["$rel_target"]=1
      printf '%s\n' "$rel_target" >> "$MTARGETS_FILE"
    fi
  done < <(awk -F'\t' '$1 == "M" { print $2 "\t" $3 "\t" $4 }' "$EVENTS_FILE")
  if [[ -s "$MTARGETS_FILE" ]]; then
    LC_ALL=C sort -o "$MTARGETS_FILE" "$MTARGETS_FILE"
  fi

  # ── 内联区域事件：剔除挂载目标（挂载文件走全文件扫，防双计）──
  if [[ -s "$MTARGETS_FILE" ]]; then
    awk 'NR == FNR { excl[$0] = 1; next } $1 == "H" && !($2 in excl)' \
      "$MTARGETS_FILE" "$EVENTS_FILE" > "$FILTERED_EVENTS_FILE"
  else
    awk -F'\t' '$1 == "H"' "$EVENTS_FILE" > "$FILTERED_EVENTS_FILE"
  fi

  # ── 扩围通道条目收集（复用同一规则数组，含 TEMP_ROOT 补充规则）──
  for i in "${!LABELS[@]}"; do
    label="${LABELS[$i]}"
    pattern="${PATTERNS[$i]}"
    mode="${MODES[$i]}"

    # A. 内联通道：cfg(test) 区域内行逐条匹配（内容列 = 第 4 字段起原文，
    #    与 grep 全行匹配语义一致；E=动态 ERE / F=index 字面量）
    while IFS= read -r entry; do
      if [[ -n "$entry" ]]; then
        ENTRIES+=("$entry")
      fi
    done < <(
      awk -F'\t' -v mode="$mode" -v pat="$pattern" -v label="$label" '
        $1 == "H" {
          content = $0
          sub(/^[^\t]*\t[^\t]*\t[^\t]*\t/, "", content)
          hit = (mode == "F") ? (index(content, pat) > 0) : (content ~ pat)
          if (hit) printf "%s:%s: %s\n", $2, $3, label
        }' "$FILTERED_EVENTS_FILE"
    )

    # B. 挂载通道：目标文件全文件 grep（与原通道同一 grep 语义）
    while IFS= read -r mfile; do
      while IFS= read -r match; do
        line="${match%%:*}"
        ENTRIES+=("${mfile}:${line}: ${label}")
      done < <(grep -n"$mode" -- "$pattern" "$scan_root/$mfile" 2>/dev/null || true)
    done < "$MTARGETS_FILE"
  done

  rm -f "$EVENTS_FILE" "$MTARGETS_FILE" "$FILTERED_EVENTS_FILE"
  EVENTS_FILE=""
  MTARGETS_FILE=""
  FILTERED_EVENTS_FILE=""
}

# ══════════════════════════════════════════════════════════
# --self-test：内嵌最小 fixture 自测（mktemp 骨架，离线，不读仓库与 baseline）
# ══════════════════════════════════════════════════════════
# 对 fixture 树调用与正常模式完全相同的 collect_static_entries 实现，
# 断言命中/不命中规格矩阵（每条断言独立报告，任一失败 → 整体 exit 1）：
#   1. 内联 cfg(test) 区域命中（mod 块内规则命中）
#   2. 生产区不命中（cfg(test) 区域外 create_dir_all 不产生条目）
#   3. #[cfg(not(test))] 区域不命中
#   4. 平铺 helper 豁免（basename 含 test_helpers / test_support 词干）
#   5. 挂载解析：#[path] 形态 + 非 mod.rs 声明者（<stem>/ 子目录）形态 +
#      单行多属性 + #[path] 组合形态
#   6. 字面量剥离：不平衡花括号 / 转义引号字面量不扰动区域边界
#      （区域闭合后的生产代码不因字面量失衡误命中）
#   7. 原 find 谓词文件（*_tests.rs）无双计（原通道恰好 1 条）
#   8. 全量命中集与期望精确一致（无任何额外命中）
#   9. 目录化 helper（路径含 /test_helpers/、/test_support/ 段）不命中
#      且不作为挂载目标（含子模块裸调用 + 内联区域）
#  10. helper 文件内含内联 cfg(test) 区域也不命中（机制保证回归）
#  11. 多行 raw string 剥离：区域内 r#"…"#（内含不平衡花括号）不截断区域，
#      其后命中不丢；raw 内容花括号不扰动深度（区域闭合后生产行不误命中）
#  12. test_support 同义词干豁免（basename ∪ 路径段另一分支）：目录化
#      test_support 不命中且不作为挂载目标、平铺 *test_support* basename 豁免
#  13. 多井号 raw string r##"…"##：井号数匹配闭合（内容中 "# 不提前闭合），
#      区域不截断、其后命中不丢，区域闭合后生产不误命中
ST_FAILS=0

# 比对键去行号归一化（与 test-naming-guard 同一归一化；baseline 比对段同用）：
# `路径:行号: 规则` → `路径: 规则`；无行号段形态原样透传
strip_key() {
  local entry="$1"
  if [[ "$entry" =~ ^(.+):[0-9]+:\ (.+)$ ]]; then
    printf '%s: %s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}"
  else
    printf '%s\n' "$entry"
  fi
}

st_line_of() {
  # fixture 内 sentinel 所在行号（1 起）；sentinel 必须唯一存在（fixture 可控）
  grep -m1 -n "$1" "$2" | cut -d: -f1
}

st_assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [[ "$expected" == "$actual" ]]; then
    echo "[self-test] ok: $desc"
  else
    echo "[self-test] FAIL: $desc" >&2
    echo "[self-test]   期望: [$expected]" >&2
    echo "[self-test]   实际: [$actual]" >&2
    ST_FAILS=$((ST_FAILS + 1))
  fi
}

st_count_entry() {
  # 统计 ENTRIES 中与 $1 完全相等的条目数
  local want="$1" n=0 entry
  for entry in "${ENTRIES[@]}"; do
    if [[ "$entry" == "$want" ]]; then
      n=$((n + 1))
    fi
  done
  printf '%s' "$n"
}

st_count_key() {
  # 统计 ENTRIES 中 strip_key 归一后等于 $1 的条目数（跨行号计数）
  local want="$1" n=0 entry
  for entry in "${ENTRIES[@]}"; do
    if [[ "$(strip_key "$entry")" == "$want" ]]; then
      n=$((n + 1))
    fi
  done
  printf '%s' "$n"
}

run_self_test() {
  echo "[self-test] 构建 fixture（mktemp）..." >&2
  local fx
  fx="$(mktemp -d "${TMPDIR:-/tmp}/test-audit-selftest-XXXXXX")"
  FIXTURE_DIR="$fx"
  mkdir -p "$fx/src" "$fx/tests" "$fx/crates/demo/src/wrapper" \
    "$fx/crates/demo/src/test_helpers" "$fx/crates/demo/src/test_support"

  cat > "$fx/crates/demo/src/mod.rs" <<'EOF'
// 挂载声明者：#[path] 形态（属性与声明跨行）+ helper 命名排除 +
// 单行多属性 + #[path] 组合形态 + 目录化 helper（路径段豁免）
#[cfg(test)]
#[path = "with_path_target.rs"]
mod with_path;

#[cfg(test)]
#[path = "demo_test_helpers.rs"]
mod helpers;

#[cfg(test)] #[path = "combo_target.rs"] mod combo;

#[cfg(test)]
mod test_helpers;

#[cfg(test)]
mod test_support;

pub fn prod_entry() {
    std::fs::create_dir_all("prod-entry"); // PROD-ENTRY-NOT-HIT
}
EOF

  cat > "$fx/crates/demo/src/inline_ok.rs" <<'EOF'
pub fn prod_top() {
    std::fs::create_dir_all("prod-top"); // PROD-TOP-NOT-HIT
}

#[cfg(test)]
mod tests {
    const BRACES: &str = "{{{ ";
    const ESCAPED: &str = "a\"}b";
    const RAW: &str = r#"
raw line one }}} } {
raw line two } "quoted }}" still raw
"#;
    fn hit_fn() {
        std::fs::create_dir_all("hit-1"); // INLINE-HIT
    }
    fn raw_region_hit() {
        std::fs::create_dir_all("hit-raw"); // RAW-REGION-HIT
    }
}

pub fn prod_after() {
    std::fs::create_dir_all("prod-after"); // PROD-AFTER-NOT-HIT
}
EOF

  cat > "$fx/crates/demo/src/not_test.rs" <<'EOF'
#[cfg(not(test))]
mod platform {
    fn no_hit() {
        std::fs::remove_dir_all("never"); // NOT-TEST-NOT-HIT
    }
}
EOF

  cat > "$fx/crates/demo/src/foo_tests.rs" <<'EOF'
#[cfg(test)]
mod inner {
    fn t() {
        std::fs::create_dir_all("orig"); // ORIG-ONCE
    }
}
EOF

  cat > "$fx/crates/demo/src/with_path_target.rs" <<'EOF'
fn mounted_hit() {
    std::fs::create_dir_all("wp"); // MOUNT-PATH-HIT
}
EOF

  cat > "$fx/crates/demo/src/combo_target.rs" <<'EOF'
fn combo_hit() {
    std::fs::create_dir_all("combo"); // COMBO-MOUNT-HIT
}
EOF

  cat > "$fx/crates/demo/src/demo_test_helpers.rs" <<'EOF'
fn helper_hit() {
    std::fs::create_dir_all("helper"); // HELPER-EXCLUDED-NOT-HIT
}

#[cfg(test)]
mod internal_tests {
    fn inline_helper_hit() {
        std::fs::create_dir_all("helper-inline"); // HELPER-INLINE-REGION-NOT-HIT
    }
}
EOF

  cat > "$fx/crates/demo/src/test_helpers/mod.rs" <<'EOF'
pub mod config;

fn mod_helper_hit() {
    std::fs::create_dir_all("dir-helper-mod"); // DIR-HELPER-MOD-NOT-HIT
}
EOF

  cat > "$fx/crates/demo/src/test_helpers/config.rs" <<'EOF'
fn dir_helper_hit() {
    std::fs::create_dir_all("dir-helper"); // DIR-HELPER-EXCLUDED-NOT-HIT
}

#[cfg(test)]
mod config_tests {
    fn inline_dir_helper_hit() {
        std::fs::create_dir_all("dir-helper-inline"); // DIR-HELPER-INLINE-NOT-HIT
    }
}
EOF

  cat > "$fx/crates/demo/src/test_support/mod.rs" <<'EOF'
fn support_hit() {
    std::fs::create_dir_all("dir-support-mod"); // DIR-SUPPORT-MOD-NOT-HIT
}

#[cfg(test)]
mod support_tests {
    fn inline_support_hit() {
        std::fs::create_dir_all("dir-support-inline"); // DIR-SUPPORT-INLINE-NOT-HIT
    }
}
EOF

  cat > "$fx/crates/demo/src/demo_test_support.rs" <<'EOF'
fn flat_support_hit() {
    std::fs::create_dir_all("flat-support"); // FLAT-SUPPORT-NOT-HIT
}
EOF

  cat > "$fx/crates/demo/src/wrapper.rs" <<'EOF'
// 非 mod.rs 声明者形态：单行多属性（循环剥离前导 #[...] 组）+ mod 挂载
#[cfg(test)] mod mounted;
EOF

  cat > "$fx/crates/demo/src/wrapper/mounted.rs" <<'EOF'
fn wrapper_hit() {
    std::fs::create_dir_all("wm"); // WRAPPER-MOUNT-HIT
}
EOF

  cat > "$fx/crates/demo/src/raw_hash.rs" <<'EOF'
#[cfg(test)]
mod tests {
    const RAW2: &str = r##"
raw2 line one }}} { {
raw2 line two "# not close }}
still raw2 {
"##;
    fn hit_fn() {
        std::fs::create_dir_all("hit-hash-raw"); // HASH-RAW-HIT
    }
}

pub fn prod_after() {
    std::fs::create_dir_all("prod-hash-after"); // PROD-HASH-AFTER-NOT-HIT
}
EOF

  # 用与正常模式完全相同的实现扫描 fixture
  collect_static_entries "$fx"

  local rel="crates/demo/src"
  # 期望行号经 sentinel 动态解析（不依赖 heredoc 排版）
  local l_inline l_after l_top l_entry l_orig l_wp l_wm l_combo l_raw l_hashraw
  l_inline="$(st_line_of 'INLINE-HIT' "$fx/$rel/inline_ok.rs")"
  l_raw="$(st_line_of 'RAW-REGION-HIT' "$fx/$rel/inline_ok.rs")"
  l_after="$(st_line_of 'PROD-AFTER-NOT-HIT' "$fx/$rel/inline_ok.rs")"
  l_top="$(st_line_of 'PROD-TOP-NOT-HIT' "$fx/$rel/inline_ok.rs")"
  l_entry="$(st_line_of 'PROD-ENTRY-NOT-HIT' "$fx/$rel/mod.rs")"
  l_orig="$(st_line_of 'ORIG-ONCE' "$fx/$rel/foo_tests.rs")"
  l_wp="$(st_line_of 'MOUNT-PATH-HIT' "$fx/$rel/with_path_target.rs")"
  l_wm="$(st_line_of 'WRAPPER-MOUNT-HIT' "$fx/$rel/wrapper/mounted.rs")"
  l_combo="$(st_line_of 'COMBO-MOUNT-HIT' "$fx/$rel/combo_target.rs")"
  l_hashraw="$(st_line_of 'HASH-RAW-HIT' "$fx/$rel/raw_hash.rs")"

  local n
  # 1. 内联 cfg(test) 区域命中
  n="$(st_count_entry "${rel}/inline_ok.rs:${l_inline}: 手动目录管理")"
  st_assert_eq "1 内联 cfg(test) 区域命中（inline_ok.rs:${l_inline}）" "1" "$n"

  # 2. 生产区不命中（inline_ok.rs 全文件恰 2 条 = 仅内联命中；mod.rs 0 条）
  n="$(st_count_key "${rel}/inline_ok.rs: 手动目录管理")"
  st_assert_eq "2a 生产区不命中（inline_ok.rs 仅 2 条内联命中）" "2" "$n"
  n="$(st_count_entry "${rel}/inline_ok.rs:${l_top}: 手动目录管理")"
  st_assert_eq "2b 生产区不命中（区域前 prod_top:${l_top}）" "0" "$n"
  n="$(st_count_entry "${rel}/mod.rs:${l_entry}: 手动目录管理")"
  st_assert_eq "2c 生产区不命中（mod.rs prod_entry:${l_entry}）" "0" "$n"

  # 3. not(test) 不命中
  n="$(st_count_entry "${rel}/not_test.rs: 手动目录管理")"
  st_assert_eq "3 not(test) 区域不命中" "0" "$n"

  # 4. 平铺 helper 豁免（basename 含 test_helpers 词干）
  n="$(st_count_entry "${rel}/demo_test_helpers.rs: 手动目录管理")"
  st_assert_eq "4 平铺 helper 豁免（demo_test_helpers.rs 裸调用不命中）" "0" "$n"

  # 5. 挂载解析（#[path]、非 mod.rs 声明者、单行多属性 + #[path] 组合三种形态）
  n="$(st_count_entry "${rel}/with_path_target.rs:${l_wp}: 手动目录管理")"
  st_assert_eq "5a 挂载解析命中（#[path] 形态 with_path_target.rs:${l_wp}）" "1" "$n"
  n="$(st_count_entry "${rel}/wrapper/mounted.rs:${l_wm}: 手动目录管理")"
  st_assert_eq "5b 挂载解析命中（非 mod.rs 声明者 wrapper/mounted.rs:${l_wm}）" "1" "$n"
  n="$(st_count_entry "${rel}/combo_target.rs:${l_combo}: 手动目录管理")"
  st_assert_eq "5c 挂载解析命中（单行多属性 + #[path] 组合 combo_target.rs:${l_combo}）" "1" "$n"

  # 6. 字面量剥离（区域闭合后生产行不因字面量失衡误命中）
  n="$(st_count_entry "${rel}/inline_ok.rs:${l_after}: 手动目录管理")"
  st_assert_eq "6 字面量剥离（区域后 prod_after:${l_after} 不误命中）" "0" "$n"

  # 7. 原 find 谓词文件无双计
  n="$(st_count_entry "${rel}/foo_tests.rs:${l_orig}: 手动目录管理")"
  st_assert_eq "7 原 find 谓词文件无双计（foo_tests.rs:${l_orig} 恰 1 条）" "1" "$n"

  # 8. 全量命中集与期望精确一致（无任何额外命中）
  local -a expected=()
  expected+=("${rel}/foo_tests.rs:${l_orig}: 手动目录管理")
  expected+=("${rel}/inline_ok.rs:${l_inline}: 手动目录管理")
  expected+=("${rel}/inline_ok.rs:${l_raw}: 手动目录管理")
  expected+=("${rel}/raw_hash.rs:${l_hashraw}: 手动目录管理")
  expected+=("${rel}/with_path_target.rs:${l_wp}: 手动目录管理")
  expected+=("${rel}/wrapper/mounted.rs:${l_wm}: 手动目录管理")
  expected+=("${rel}/combo_target.rs:${l_combo}: 手动目录管理")
  local expected_sorted actual_sorted=""
  expected_sorted="$(printf '%s\n' "${expected[@]}" | LC_ALL=C sort)"
  if [[ ${#ENTRIES[@]} -gt 0 ]]; then
    actual_sorted="$(printf '%s\n' "${ENTRIES[@]}" | LC_ALL=C sort)"
  fi
  st_assert_eq "8 全量命中集与期望精确一致（共 7 条，无额外命中）" \
    "$expected_sorted" "$actual_sorted"

  # 9. 目录化 helper（路径含 /test_helpers/、/test_support/ 段）不命中且不作为挂载目标
  n="$(st_count_key "${rel}/test_helpers/config.rs: 手动目录管理")"
  st_assert_eq "9a 目录化 helper 子模块不命中（test_helpers/config.rs 裸调用）" "0" "$n"
  n="$(st_count_key "${rel}/test_helpers/mod.rs: 手动目录管理")"
  st_assert_eq "9b 目录化 helper 不作为挂载目标（test_helpers/mod.rs 不入全文件扫）" "0" "$n"
  n="$(st_count_key "${rel}/test_support/mod.rs: 手动目录管理")"
  st_assert_eq "9c 目录化 test_support 不命中且不作为挂载目标（test_support/mod.rs）" "0" "$n"

  # 10. helper 文件内含内联 cfg(test) 区域也不命中（机制保证回归）
  n="$(st_count_key "${rel}/demo_test_helpers.rs: 手动目录管理")"
  st_assert_eq "10 helper 文件内联 cfg(test) 区域不命中（demo_test_helpers.rs）" "0" "$n"

  # 11. 多行 raw string 剥离（区域内 r#"…"# 不截断区域，其后命中不丢；
  #     raw 内容花括号不扰动深度——区域闭合后 prod_after 不误命中由 6/8 守护）
  n="$(st_count_entry "${rel}/inline_ok.rs:${l_raw}: 手动目录管理")"
  st_assert_eq "11 raw string 后命中不丢（区域不提前弹栈 inline_ok.rs:${l_raw}）" "1" "$n"

  # 12. test_support 同义词干豁免（basename ∪ 路径段并集的另一分支）
  n="$(st_count_key "${rel}/demo_test_support.rs: 手动目录管理")"
  st_assert_eq "12 平铺 test_support 词干豁免（demo_test_support.rs 裸调用不命中）" "0" "$n"

  # 13. 多井号 raw string r##"…"##（内容中 "# 不提前闭合；区域闭合后
  #     prod-hash-after 不误命中由 8 守护）
  n="$(st_count_entry "${rel}/raw_hash.rs:${l_hashraw}: 手动目录管理")"
  st_assert_eq "13 多井号 raw string 后命中不丢（区域不截断 raw_hash.rs:${l_hashraw}）" "1" "$n"
  n="$(st_count_key "${rel}/raw_hash.rs: 手动目录管理")"
  st_assert_eq "13b 多井号 raw string 区域闭合后生产不误命中（raw_hash.rs 仅 1 条）" "1" "$n"

  rm -rf "$fx"
  FIXTURE_DIR=""
  if [[ $ST_FAILS -gt 0 ]]; then
    echo "[self-test] 失败 — ${ST_FAILS} 项断言未通过" >&2
    exit 1
  fi
  echo "[self-test] 通过 — 全部断言通过（规格矩阵 13 组）"
  exit 0
}

if [[ $SELF_TEST -eq 1 ]]; then
  run_self_test
fi

collect_static_entries "$PROJECT_ROOT"

# 排序但保留重复条目（C locale 字节序，保证与 baseline 比对及重写结果确定性）：
# 同一行可同时命中 '"/tmp 与 "$TEMP_ROOT 补充规则（同 label「硬编码路径」），产生重复条目；
# 写侧塌缩会与加载/比对侧多重集计数不对称，破坏重写→加载→比对闭环自洽
ACTUAL=()
if [[ ${#ENTRIES[@]} -gt 0 ]]; then
  mapfile -t ACTUAL < <(printf '%s\n' "${ENTRIES[@]}" | LC_ALL=C sort)
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
# 新增（baseline 外，或同键实际命中数超出 baseline 计数的差额）→ 列出并 fail；
# 存量（baseline 计数覆盖的部分）→ 豁免；已消除（baseline 有、实际无，或同键
# 缺额的差额）→ 打「可收窄」警告（不 fail）。
# --target 过滤模式下跳过「已消除」判定（过滤子集无法证明全量消除）。
# 比对键去行号（strip_key，与 test-naming-guard 同一归一化）：`路径:行号: 规则`
# → `路径: 规则`；违规输出与 baseline 重写格式保持含行号不变，仅比对时归一化，
# 行号漂移不误报 FAIL。无行号段形态原样透传。
# 比对为多重集（计数）语义，与 test_naming_guard.py Counter 语义一致：同键
# 多条目并存不塌缩——实际计数 − baseline 计数 = 新增差额（FAIL），baseline
# 计数 − 实际计数 = 已消除差额（提示）；每键报告一条代表条目（保留行号定位）。

# 比对键去行号归一化 strip_key 定义见 --self-test 段前（两处共用）

BASELINE=()
if [[ -f "$BASELINE_FILE" ]]; then
  # 不去重：多重集比对按行计数，重复条目各自占一个名额
  mapfile -t BASELINE < <(
    sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' "$BASELINE_FILE" \
      | grep -vE '^(#|$)' | LC_ALL=C sort
  )
fi

declare -A BASELINE_COUNTS=()
declare -A BASELINE_REPR=()
for entry in "${BASELINE[@]}"; do
  key="$(strip_key "$entry")"
  BASELINE_COUNTS["$key"]=$(( ${BASELINE_COUNTS["$key"]:-0} + 1 ))
  BASELINE_REPR["$key"]="${BASELINE_REPR["$key"]:-$entry}"
done

declare -A ACTUAL_COUNTS=()
declare -A ACTUAL_REPR=()
for entry in "${ACTUAL[@]}"; do
  key="$(strip_key "$entry")"
  ACTUAL_COUNTS["$key"]=$(( ${ACTUAL_COUNTS["$key"]:-0} + 1 ))
  ACTUAL_REPR["$key"]="${ACTUAL_REPR["$key"]:-$entry}"
done

NEW_VIOLATIONS=()
NEW_COUNT=0
EXEMPT_COUNT=0
for key in "${!ACTUAL_COUNTS[@]}"; do
  base_n="${BASELINE_COUNTS["$key"]:-0}"
  act_n="${ACTUAL_COUNTS["$key"]}"
  if (( act_n > base_n )); then
    NEW_COUNT=$((NEW_COUNT + act_n - base_n))
    NEW_VIOLATIONS+=("${ACTUAL_REPR["$key"]}")
  fi
  if (( act_n < base_n )); then
    EXEMPT_COUNT=$((EXEMPT_COUNT + act_n))
  else
    EXEMPT_COUNT=$((EXEMPT_COUNT + base_n))
  fi
done
if [[ ${#NEW_VIOLATIONS[@]} -gt 0 ]]; then
  mapfile -t NEW_VIOLATIONS < <(printf '%s\n' "${NEW_VIOLATIONS[@]}" | LC_ALL=C sort)
fi

ELIMINATED=()
ELIMINATED_COUNT=0
if [[ -f "$BASELINE_FILE" && -z "$TARGET_FILTER" ]]; then
  for key in "${!BASELINE_COUNTS[@]}"; do
    base_n="${BASELINE_COUNTS["$key"]}"
    act_n="${ACTUAL_COUNTS["$key"]:-0}"
    if (( base_n > act_n )); then
      ELIMINATED_COUNT=$((ELIMINATED_COUNT + base_n - act_n))
      ELIMINATED+=("${BASELINE_REPR["$key"]}")
    fi
  done
  if [[ ${#ELIMINATED[@]} -gt 0 ]]; then
    mapfile -t ELIMINATED < <(printf '%s\n' "${ELIMINATED[@]}" | LC_ALL=C sort)
  fi
fi

# ── 结果 ──────────────────────────────────────────────────
if [[ $NEW_COUNT -gt 0 ]]; then
  printf '%s\n' "${NEW_VIOLATIONS[@]}"
fi

if [[ ${#ELIMINATED[@]} -gt 0 ]]; then
  echo "[test-audit] ${ELIMINATED_COUNT} 条 baseline 条目已消除，可收窄（bash scripts/test-audit.sh --update-baseline 重写）:" >&2
  printf '  %s\n' "${ELIMINATED[@]}" >&2
fi

if [[ $NEW_COUNT -gt 0 || $VIOLATIONS -gt 0 ]]; then
  echo "[test-audit] 失败 — baseline 外新增违规 ${NEW_COUNT} 处，运行时违规 ${VIOLATIONS} 处，baseline 豁免 ${EXEMPT_COUNT} 处" >&2
  exit 1
fi

echo "[test-audit] 通过 — 静态命中 ${#ACTUAL[@]} 处全部在 baseline 内（豁免 ${EXEMPT_COUNT} 处），已消除 ${ELIMINATED_COUNT} 条（可 --update-baseline 收窄），运行时违规 0" >&2
exit 0
