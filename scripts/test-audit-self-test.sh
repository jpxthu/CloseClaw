#!/usr/bin/env bash
#
# test-audit-self-test.sh — test-audit.sh 扫描器自测（fixture 构建/断言/规格矩阵）
# 用法: bash scripts/test-audit.sh --self-test   （对外入口，主脚本转调本脚本）
#       bash scripts/test-audit-self-test.sh     （直接调用等价）
#
# 对 mktemp 内嵌最小 fixture 树运行与正常模式完全相同的扫描实现——库模式
# source test-audit.sh（TEST_AUDIT_LIB_MODE=1，仅取定义不执行），复用
# REGION_SCANNER_AWK / is_helper_exempt / collect_static_entries，保证
# 「测的就是真实扫描逻辑」；断言命中/不命中规格矩阵（每条断言独立报告，
# 任一失败 → 整体 exit 1）；离线、不读仓库与 baseline。
#
# 规格矩阵（17 组）：
#   1. 内联 cfg(test) 区域命中（mod 块内规则命中）
#   2. 生产区不命中（cfg(test) 区域外 create_dir_all 不产生条目）
#   3. #[cfg(not(test))] 区域不命中
#   4. 平铺 helper 豁免（basename 含 test_helpers / test_support 词干；
#      fixture 仅含裸调用无内联区域，失败可归因挂载通道）
#   5. 挂载解析：#[path] 形态 + 非 mod.rs 声明者（<stem>/ 子目录）形态 +
#      单行多属性 + #[path] 组合形态
#   6. 字面量剥离：不平衡花括号 / 转义引号字面量不扰动区域边界
#      （区域闭合后的生产代码不因字面量失衡误命中）
#   7. 原 find 谓词文件（*_tests.rs）无双计（原通道恰好 1 条）
#   8. 全量命中集与期望精确一致（无任何额外命中）
#   9. 目录化 helper（路径含 /test_helpers/、/test_support/ 段）不命中
#      且不作为挂载目标（含子模块裸调用 + 内联区域）
#  10. helper 文件内含内联 cfg(test) 区域也不命中（机制保证回归；fixture
#      仅含内联区域无裸调用，失败可归因内联区域通道，与 4 判别力分离）
#  11. 多行 raw string 剥离：区域内 r#"…"#（内含不平衡花括号）不截断区域，
#      其后命中不丢；raw 内容花括号不扰动深度（区域闭合后生产行不误命中）
#  12. test_support 同义词干豁免（basename ∪ 路径段另一分支）：目录化
#      test_support 不命中且不作为挂载目标、平铺 *test_support* basename 豁免
#  13. 多井号 raw string r##"…"##：井号数匹配闭合（内容中 "# 不提前闭合），
#      区域不截断、其后命中不丢，区域闭合后生产不误命中
#  14. raw string 体内 //（// 剥离字符串感知回归）：单行闭合同行形态
#      （如 r#"see https://x"#）与跨行闭合定界符同行形态 rs_open 均不悬挂
#      ——区域内命中不丢、区域后生产不误命中
#  15. 0 井 raw string r"…" 多行形态：rs_hash=0 独立分支闭合正确——区域
#      不截断、其后命中不丢、区域闭合后生产不误命中
#  16. 豁免首段形态（相对 scan_root 一级 test_helpers/，挂载通道 #[path]
#      相对上跳可达）：不作为挂载目标、不命中（路径段豁免含首段）
#  17. 判据收窄回归：*_tests.rs 后缀真实测试文件永不豁免（无论词干/路径段
#      命中）——词干命中形态 test_helpers_tests.rs 与路径段命中形态
#      test_helpers/config_tests.rs 恢复扫描，区域内裸违规正常命中
#
# 文件拆分说明：test-audit.sh 曾因内嵌 self-test 超 CONTRIBUTING 文件行数
# 上限 1000，run_self_test 单函数超函数体 100 行限制；拆至本文件并将
# fixture 构建 / 断言段按文件族 / 规格组抽为独立函数收敛函数体

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── 库模式加载主脚本（仅定义不执行，见 test-audit.sh 库模式出口）──
export TEST_AUDIT_LIB_MODE=1
# shellcheck disable=SC1091
source "$SCRIPT_DIR/test-audit.sh"

ST_FAILS=0

# ── 断言 helper ──────────────────────────────────────────
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

# ── fixture 构建（mktemp 骨架，按文件族分段）──────────────

# 挂载声明者文件：#[path] 形态（属性与声明跨行）+ helper 命名排除 +
# 单行多属性 + #[path] 组合形态 + 非 mod.rs 声明者（wrapper.rs）
st_fixture_mount_declares() {
  local fx="$1"
  cat > "$fx/crates/demo/src/mod.rs" <<'EOF'
// 挂载声明者：#[path] 形态（属性与声明跨行）+ helper 命名排除 +
// 单行多属性 + #[path] 组合形态 + 目录化 helper（路径段豁免）+
// #[path] 相对上跳至 scan_root 首段 helper（首段豁免）
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

#[cfg(test)]
#[path = "../../../test_helpers/root_helper.rs"]
mod root_helpers;

pub fn prod_entry() {
    std::fs::create_dir_all("prod-entry"); // PROD-ENTRY-NOT-HIT
}
EOF

  cat > "$fx/crates/demo/src/wrapper.rs" <<'EOF'
// 非 mod.rs 声明者形态：单行多属性（循环剥离前导 #[...] 组）+ mod 挂载
#[cfg(test)] mod mounted;
EOF
}

# 内联区域与原谓词文件：生产区/内联区对照、not(test) 排除、原通道无双计、
# 字面量剥离（不平衡花括号 / 转义引号）+ 多行 raw string 区域不截断
st_fixture_inline_regions() {
  local fx="$1"
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
}

# raw string 形态族：多井号 r##"…"##、raw 体内 //（单行同行闭合 + 跨行闭合
# 定界符同行）、0 井 r"…" 多行（rs_hash=0 独立分支）
st_fixture_raw_forms() {
  local fx="$1"
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

  cat > "$fx/crates/demo/src/raw_slash.rs" <<'EOF'
pub fn prod_top() {
    std::fs::create_dir_all("prod-slash-top"); // PROD-SLASH-TOP-NOT-HIT
}

#[cfg(test)]
mod tests {
    const URL: &str = r#"see https://x"#;
    fn hit_one() {
        std::fs::create_dir_all("hit-slash-inline"); // SLASH-INLINE-HIT
    }
    const DOC: &str = r#"
raw doc line }}} {
tail // "#;
    fn hit_two() {
        std::fs::create_dir_all("hit-slash-ml"); // SLASH-ML-HIT
    }
}

pub fn prod_after() {
    std::fs::create_dir_all("prod-slash-after"); // PROD-SLASH-AFTER-NOT-HIT
}
EOF

  cat > "$fx/crates/demo/src/raw_zero.rs" <<'EOF'
pub fn prod_top() {
    std::fs::create_dir_all("prod-r0-top"); // PROD-R0-TOP-NOT-HIT
}

#[cfg(test)]
mod tests {
    const R0: &str = r"
zero-hash raw }}} {
";
    fn hit_fn() {
        std::fs::create_dir_all("hit-r0"); // R0-HIT
    }
}

pub fn prod_after() {
    std::fs::create_dir_all("prod-r0-after"); // PROD-R0-AFTER-NOT-HIT
}
EOF
}

# 挂载目标文件：#[path] 目标 + 单行多属性组合目标 + 非 mod.rs 声明者子目录目标
st_fixture_mount_targets() {
  local fx="$1"
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

  cat > "$fx/crates/demo/src/wrapper/mounted.rs" <<'EOF'
fn wrapper_hit() {
    std::fs::create_dir_all("wm"); // WRAPPER-MOUNT-HIT
}
EOF
}

# helper 本体族：平铺命名（basename 词干）+ 目录化子模块（路径段）+
# test_support 同义词干 + scan_root 首段目录；含内联 cfg(test) 区域形态
# （机制保证回归：helper 文件内内联区域也不命中，fixture 仅含内联区域
# 无裸调用，失败可归因内联区域通道）；*_tests.rs 真实测试文件（词干/
# 路径段命中形态）永不豁免（判据收窄回归）
st_fixture_helpers() {
  local fx="$1"
  cat > "$fx/crates/demo/src/demo_test_helpers.rs" <<'EOF'
fn helper_hit() {
    std::fs::create_dir_all("helper"); // HELPER-EXCLUDED-NOT-HIT
}
EOF

  # 规格 10 专用：仅含内联 cfg(test) 区域、无裸调用（与规格 4 判别力分离）
  cat > "$fx/crates/demo/src/demo_test_helpers_inline.rs" <<'EOF'
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

  cat > "$fx/test_helpers/root_helper.rs" <<'EOF'
fn root_helper_hit() {
    std::fs::create_dir_all("root-helper"); // ROOT-HELPER-NOT-HIT
}

#[cfg(test)]
mod root_helper_tests {
    fn inline_root_helper_hit() {
        std::fs::create_dir_all("root-helper-inline"); // ROOT-HELPER-INLINE-NOT-HIT
    }
}
EOF

  # 规格 17：*_tests.rs 后缀 + 词干命中形态（原通道扫描，裸违规命中）
  cat > "$fx/crates/demo/src/test_helpers_tests.rs" <<'EOF'
#[cfg(test)]
mod stem_tests {
    fn stem_tests_hit() {
        std::fs::create_dir_all("stem-tests"); // STEM-TESTS-HIT
    }
}
EOF

  # 规格 17：*_tests.rs 后缀 + 路径段命中形态（原通道扫描，裸违规命中）
  cat > "$fx/crates/demo/src/test_helpers/config_tests.rs" <<'EOF'
#[cfg(test)]
mod config_file_tests {
    fn seg_tests_hit() {
        std::fs::create_dir_all("seg-tests"); // SEG-TESTS-HIT
    }
}
EOF
}

st_build_fixtures() {
  local fx="$1"
  mkdir -p "$fx/src" "$fx/tests" "$fx/crates/demo/src/wrapper" \
    "$fx/crates/demo/src/test_helpers" "$fx/crates/demo/src/test_support" \
    "$fx/test_helpers"
  st_fixture_mount_declares "$fx"
  st_fixture_inline_regions "$fx"
  st_fixture_raw_forms "$fx"
  st_fixture_mount_targets "$fx"
  st_fixture_helpers "$fx"
}

# ── 期望行号解析（sentinel → 行号，写入全局 ST_L_*）───────
# fixture 内 sentinel 唯一存在（fixture 可控），不依赖 heredoc 排版
st_resolve_expected_lines() {
  local fx="$1" rel="crates/demo/src"
  ST_L_INLINE="$(st_line_of 'INLINE-HIT' "$fx/$rel/inline_ok.rs")"
  ST_L_RAW="$(st_line_of 'RAW-REGION-HIT' "$fx/$rel/inline_ok.rs")"
  ST_L_AFTER="$(st_line_of 'PROD-AFTER-NOT-HIT' "$fx/$rel/inline_ok.rs")"
  ST_L_TOP="$(st_line_of 'PROD-TOP-NOT-HIT' "$fx/$rel/inline_ok.rs")"
  ST_L_ENTRY="$(st_line_of 'PROD-ENTRY-NOT-HIT' "$fx/$rel/mod.rs")"
  ST_L_ORIG="$(st_line_of 'ORIG-ONCE' "$fx/$rel/foo_tests.rs")"
  ST_L_WP="$(st_line_of 'MOUNT-PATH-HIT' "$fx/$rel/with_path_target.rs")"
  ST_L_WM="$(st_line_of 'WRAPPER-MOUNT-HIT' "$fx/$rel/wrapper/mounted.rs")"
  ST_L_COMBO="$(st_line_of 'COMBO-MOUNT-HIT' "$fx/$rel/combo_target.rs")"
  ST_L_HASHRAW="$(st_line_of 'HASH-RAW-HIT' "$fx/$rel/raw_hash.rs")"
  ST_L_SLASH1="$(st_line_of 'SLASH-INLINE-HIT' "$fx/$rel/raw_slash.rs")"
  ST_L_SLASH2="$(st_line_of 'SLASH-ML-HIT' "$fx/$rel/raw_slash.rs")"
  ST_L_R0="$(st_line_of 'R0-HIT' "$fx/$rel/raw_zero.rs")"
  ST_L_STEM_TESTS="$(st_line_of 'STEM-TESTS-HIT' "$fx/$rel/test_helpers_tests.rs")"
  ST_L_SEG_TESTS="$(st_line_of 'SEG-TESTS-HIT' "$fx/$rel/test_helpers/config_tests.rs")"
}

# ── 断言段（规格矩阵分组，各组独立报告，失败计数入 ST_FAILS）──

# 组 1-8：核心通道（内联区域命中 / 生产区不命中 / not(test) / 平铺 helper /
# 挂载解析三形态 / 字面量剥离 / 原谓词无双计 / 全量命中集精确一致）
st_assert_core_specs() {
  local n rel="crates/demo/src"
  # 1. 内联 cfg(test) 区域命中
  n="$(st_count_entry "${rel}/inline_ok.rs:${ST_L_INLINE}: 手动目录管理")"
  st_assert_eq "1 内联 cfg(test) 区域命中（inline_ok.rs:${ST_L_INLINE}）" "1" "$n"

  # 2. 生产区不命中（inline_ok.rs 全文件恰 2 条 = 仅内联命中；mod.rs 0 条）
  n="$(st_count_key "${rel}/inline_ok.rs: 手动目录管理")"
  st_assert_eq "2a 生产区不命中（inline_ok.rs 仅 2 条内联命中）" "2" "$n"
  n="$(st_count_entry "${rel}/inline_ok.rs:${ST_L_TOP}: 手动目录管理")"
  st_assert_eq "2b 生产区不命中（区域前 prod_top:${ST_L_TOP}）" "0" "$n"
  n="$(st_count_entry "${rel}/mod.rs:${ST_L_ENTRY}: 手动目录管理")"
  st_assert_eq "2c 生产区不命中（mod.rs prod_entry:${ST_L_ENTRY}）" "0" "$n"

  # 3. not(test) 不命中（无行号形态经 st_count_key 归一化跨行号计数；
  #    st_count_entry 对无行号 want 恒不命中、断言无效力）
  n="$(st_count_key "${rel}/not_test.rs: 手动目录管理")"
  st_assert_eq "3 not(test) 区域不命中" "0" "$n"

  # 4. 平铺 helper 豁免（basename 含 test_helpers 词干；同 3 用 st_count_key，
  #    豁免失效时真实 FAIL；fixture 仅含裸调用无内联区域——该文件是 #[path]
  #    挂载目标，失败可归因挂载通道）
  n="$(st_count_key "${rel}/demo_test_helpers.rs: 手动目录管理")"
  st_assert_eq "4 平铺 helper 豁免（demo_test_helpers.rs 裸调用不命中）" "0" "$n"

  # 5. 挂载解析（#[path]、非 mod.rs 声明者、单行多属性 + #[path] 组合三种形态）
  n="$(st_count_entry "${rel}/with_path_target.rs:${ST_L_WP}: 手动目录管理")"
  st_assert_eq "5a 挂载解析命中（#[path] 形态 with_path_target.rs:${ST_L_WP}）" "1" "$n"
  n="$(st_count_entry "${rel}/wrapper/mounted.rs:${ST_L_WM}: 手动目录管理")"
  st_assert_eq "5b 挂载解析命中（非 mod.rs 声明者 wrapper/mounted.rs:${ST_L_WM}）" "1" "$n"
  n="$(st_count_entry "${rel}/combo_target.rs:${ST_L_COMBO}: 手动目录管理")"
  st_assert_eq "5c 挂载解析命中（单行多属性 + #[path] 组合 combo_target.rs:${ST_L_COMBO}）" "1" "$n"

  # 6. 字面量剥离（区域闭合后生产行不因字面量失衡误命中）
  n="$(st_count_entry "${rel}/inline_ok.rs:${ST_L_AFTER}: 手动目录管理")"
  st_assert_eq "6 字面量剥离（区域后 prod_after:${ST_L_AFTER} 不误命中）" "0" "$n"

  # 7. 原 find 谓词文件无双计
  n="$(st_count_entry "${rel}/foo_tests.rs:${ST_L_ORIG}: 手动目录管理")"
  st_assert_eq "7 原 find 谓词文件无双计（foo_tests.rs:${ST_L_ORIG} 恰 1 条）" "1" "$n"

  # 8. 全量命中集与期望精确一致（无任何额外命中）
  local -a expected=()
  expected+=("${rel}/foo_tests.rs:${ST_L_ORIG}: 手动目录管理")
  expected+=("${rel}/inline_ok.rs:${ST_L_INLINE}: 手动目录管理")
  expected+=("${rel}/inline_ok.rs:${ST_L_RAW}: 手动目录管理")
  expected+=("${rel}/raw_hash.rs:${ST_L_HASHRAW}: 手动目录管理")
  expected+=("${rel}/raw_slash.rs:${ST_L_SLASH1}: 手动目录管理")
  expected+=("${rel}/raw_slash.rs:${ST_L_SLASH2}: 手动目录管理")
  expected+=("${rel}/raw_zero.rs:${ST_L_R0}: 手动目录管理")
  expected+=("${rel}/with_path_target.rs:${ST_L_WP}: 手动目录管理")
  expected+=("${rel}/wrapper/mounted.rs:${ST_L_WM}: 手动目录管理")
  expected+=("${rel}/combo_target.rs:${ST_L_COMBO}: 手动目录管理")
  expected+=("${rel}/test_helpers_tests.rs:${ST_L_STEM_TESTS}: 手动目录管理")
  expected+=("${rel}/test_helpers/config_tests.rs:${ST_L_SEG_TESTS}: 手动目录管理")
  local expected_sorted actual_sorted=""
  expected_sorted="$(printf '%s\n' "${expected[@]}" | LC_ALL=C sort)"
  if [[ ${#ENTRIES[@]} -gt 0 ]]; then
    actual_sorted="$(printf '%s\n' "${ENTRIES[@]}" | LC_ALL=C sort)"
  fi
  st_assert_eq "8 全量命中集与期望精确一致（共 12 条，无额外命中）" \
    "$expected_sorted" "$actual_sorted"
}

# 组 9-16：helper 豁免机制保证（目录化 / 内联区域 / test_support / 首段）+
# raw string 剥离回归（多行 / 多井号 / 体内 // / 0 井多行）
st_assert_exemption_raw_specs() {
  local n rel="crates/demo/src"
  # 9. 目录化 helper（路径含 /test_helpers/、/test_support/ 段）不命中且不作为挂载目标
  n="$(st_count_key "${rel}/test_helpers/config.rs: 手动目录管理")"
  st_assert_eq "9a 目录化 helper 子模块不命中（test_helpers/config.rs 裸调用）" "0" "$n"
  n="$(st_count_key "${rel}/test_helpers/mod.rs: 手动目录管理")"
  st_assert_eq "9b 目录化 helper 不作为挂载目标（test_helpers/mod.rs 不入全文件扫）" "0" "$n"
  n="$(st_count_key "${rel}/test_support/mod.rs: 手动目录管理")"
  st_assert_eq "9c 目录化 test_support 不命中且不作为挂载目标（test_support/mod.rs）" "0" "$n"

  # 10. helper 文件内含内联 cfg(test) 区域也不命中（机制保证回归；fixture
  #     仅含内联区域无裸调用，失败可归因内联区域通道，与 4 判别力分离）
  n="$(st_count_key "${rel}/demo_test_helpers_inline.rs: 手动目录管理")"
  st_assert_eq "10 helper 文件内联 cfg(test) 区域不命中（demo_test_helpers_inline.rs）" "0" "$n"

  # 11. 多行 raw string 剥离（区域内 r#"…"# 不截断区域，其后命中不丢；
  #     raw 内容花括号不扰动深度——区域闭合后 prod_after 不误命中由 6/8 守护）
  n="$(st_count_entry "${rel}/inline_ok.rs:${ST_L_RAW}: 手动目录管理")"
  st_assert_eq "11 raw string 后命中不丢（区域不提前弹栈 inline_ok.rs:${ST_L_RAW}）" "1" "$n"

  # 12. test_support 同义词干豁免（basename ∪ 路径段并集的另一分支）
  n="$(st_count_key "${rel}/demo_test_support.rs: 手动目录管理")"
  st_assert_eq "12 平铺 test_support 词干豁免（demo_test_support.rs 裸调用不命中）" "0" "$n"

  # 13. 多井号 raw string r##"…"##（内容中 "# 不提前闭合；区域闭合后
  #     prod-hash-after 不误命中由 8 守护）
  n="$(st_count_entry "${rel}/raw_hash.rs:${ST_L_HASHRAW}: 手动目录管理")"
  st_assert_eq "13 多井号 raw string 后命中不丢（区域不截断 raw_hash.rs:${ST_L_HASHRAW}）" "1" "$n"
  n="$(st_count_key "${rel}/raw_hash.rs: 手动目录管理")"
  st_assert_eq "13b 多井号 raw string 区域闭合后生产不误命中（raw_hash.rs 仅 1 条）" "1" "$n"

  # 14. raw string 体内 //（// 剥离字符串感知回归）：单行闭合同行形态
  #     （r#"see https://x"#）与跨行闭合定界符同行形态 rs_open 均不悬挂；
  #     区域边界正确（区域后生产不误命中）由 14c 守护
  n="$(st_count_entry "${rel}/raw_slash.rs:${ST_L_SLASH1}: 手动目录管理")"
  st_assert_eq "14 raw 体内 // 单行同行闭合命中不丢（raw_slash.rs:${ST_L_SLASH1}）" "1" "$n"
  n="$(st_count_entry "${rel}/raw_slash.rs:${ST_L_SLASH2}: 手动目录管理")"
  st_assert_eq "14b raw 跨行体 // 闭合同行命中不丢（raw_slash.rs:${ST_L_SLASH2}）" "1" "$n"
  n="$(st_count_key "${rel}/raw_slash.rs: 手动目录管理")"
  st_assert_eq "14c raw 体内 // 区域边界正确（raw_slash.rs 恰 2 条，区域后生产不误命中）" "2" "$n"

  # 15. 0 井 raw string r"…" 多行形态（rs_hash=0 独立分支回归）：闭合正确
  #     区域不截断、其后命中不丢，区域闭合后生产不误命中
  n="$(st_count_entry "${rel}/raw_zero.rs:${ST_L_R0}: 手动目录管理")"
  st_assert_eq "15 0 井 raw string 多行后命中不丢（raw_zero.rs:${ST_L_R0}）" "1" "$n"
  n="$(st_count_key "${rel}/raw_zero.rs: 手动目录管理")"
  st_assert_eq "15b 0 井 raw string 区域闭合后生产不误命中（raw_zero.rs 仅 1 条）" "1" "$n"

  # 16. 豁免首段形态（相对 scan_root 一级 test_helpers/，挂载通道 #[path]
  #     相对上跳可达）：不作为挂载目标、不命中（路径段豁免含首段）
  n="$(st_count_key "test_helpers/root_helper.rs: 手动目录管理")"
  st_assert_eq "16 豁免首段形态不作为挂载目标且不命中（test_helpers/root_helper.rs）" "0" "$n"

  # 17. 判据收窄回归：*_tests.rs 后缀真实测试文件永不豁免（无论词干/路径段
  #     命中）——恢复扫描视野（原通道），区域内裸违规正常命中
  n="$(st_count_entry "${rel}/test_helpers_tests.rs:${ST_L_STEM_TESTS}: 手动目录管理")"
  st_assert_eq "17a 词干命中形态 *_tests.rs 不豁免（test_helpers_tests.rs:${ST_L_STEM_TESTS} 命中）" "1" "$n"
  n="$(st_count_entry "${rel}/test_helpers/config_tests.rs:${ST_L_SEG_TESTS}: 手动目录管理")"
  st_assert_eq "17b 路径段命中形态 *_tests.rs 不豁免（test_helpers/config_tests.rs:${ST_L_SEG_TESTS} 命中）" "1" "$n"
  n="$(st_count_key "${rel}/test_helpers_tests.rs: 手动目录管理")"
  st_assert_eq "17c 词干命中形态 *_tests.rs 恰 1 条（原通道无双计）" "1" "$n"
}

# ── 编排 ─────────────────────────────────────────────────
run_self_test() {
  echo "[self-test] 构建 fixture（mktemp）..." >&2
  local fx
  fx="$(mktemp -d "${TMPDIR:-/tmp}/test-audit-selftest-XXXXXX")"
  FIXTURE_DIR="$fx"
  st_build_fixtures "$fx"

  # 用与正常模式完全相同的实现扫描 fixture（库模式复用主脚本扫描实现）
  collect_static_entries "$fx"

  st_resolve_expected_lines "$fx"
  st_assert_core_specs
  st_assert_exemption_raw_specs

  rm -rf "$fx"
  FIXTURE_DIR=""
  if [[ $ST_FAILS -gt 0 ]]; then
    echo "[self-test] 失败 — ${ST_FAILS} 项断言未通过" >&2
    exit 1
  fi
  echo "[self-test] 通过 — 全部断言通过（规格矩阵 17 组）"
  exit 0
}

run_self_test
