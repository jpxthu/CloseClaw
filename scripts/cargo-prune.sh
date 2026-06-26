#!/bin/bash
# cargo-prune.sh — 清理 target/debug/deps 中的多版本 artifact，保留最新
#
# 用途: EDA workflow PR merge 后执行，替代 cargo clean
# 原理: 同一 crate 的不同版本以 hash 后缀区分，只保留每个 crate 最新文件
# 兜底: 磁盘可用空间 < 5GB 时退回 cargo clean

set -euo pipefail

PROJECT_ROOT="/home/admin/code/closeclaw"
DEPS_DIR="$PROJECT_ROOT/target/debug/deps"
DISK_AVAIL_KB=$(df -k / | awk 'NR==2 {print $4}')
DISK_AVAIL_GB=$((DISK_AVAIL_KB / 1024 / 1024))

echo "[cargo-prune] disk available: ${DISK_AVAIL_GB}GB"

# 兜底：磁盘空间不足，直接 cargo clean
if [ "$DISK_AVAIL_GB" -lt 5 ]; then
  echo "[cargo-prune] WARNING: disk < 5GB, running cargo clean instead"
  cd "$PROJECT_ROOT" && cargo clean
  post_avail=$(df -m / | awk 'NR==2 {print $4}')
  post_gb=$((post_avail / 1024))
  echo "[cargo-prune] cargo clean done, disk now ${post_gb}GB available"
  exit 0
fi

# 正常清理：删除多版本 artifact
if [ ! -d "$DEPS_DIR" ]; then
  echo "[cargo-prune] No target dir found, skipping."
  exit 0
fi

before_mb=$(du -sm "$DEPS_DIR" | cut -f1)

# 对同一 crate（去掉 hash 后缀后的同名），只保留最新的文件（按 mtime 降序）
# 扩展名分组处理：.rlib / .rmeta / .so / .d
for ext in rlib rmeta so d; do
  find "$DEPS_DIR" -name "*.${ext}" -printf '%T@ %p\n' 2>/dev/null | \
    sort -rn | \
    awk -v ext="$ext" '{
      # 提取 crate 名（去掉路径 + hash 后缀 + 扩展名）
      file=$2
      n=split(file, parts, "/")
      base=parts[n]
      # 去掉 .{ext} 后缀
      sub(/\.[^.]*$/, "", base)
      # 去掉 hash 后缀（16位以上 hex）
      sub(/-[a-f0-9]{16,}$/, "", base)
      if (!(base in seen)) {
        seen[base]=1
      } else {
        print file
      }
    }' | while read -r stale; do
      rm -f "$stale"
    done
done

# 清理孤儿 .rcgu.o 文件（对应的主体文件已删）
find "$DEPS_DIR" -name "*.rcgu.o" 2>/dev/null | while read -r o; do
  prefix=$(basename "$o" | sed 's/\.[A-Za-z0-9_]*\.rcgu\.o$//')
  if ! find "$DEPS_DIR" -name "${prefix}*" ! -name "*.rcgu.o" -quit 2>/dev/null | grep -q .; then
    rm -f "$o"
  fi
done

# 清理 incremental cache（旧分支的编译缓存）
rm -rf "$PROJECT_ROOT/target/debug/incremental/"

after_mb=$(du -sm "$DEPS_DIR" | cut -f1)
freed=$((before_mb - after_mb))
echo "[cargo-prune] deps: ${before_mb}MB -> ${after_mb}MB (freed ${freed}MB)"
