#!/usr/bin/env bash
# ============================================================
# LLM Fixture 采集入口 — Phase 1 重构
#
# 用法:
#   ./run_capture.sh <provider> <model> <protocol> <api_key>
#
# 示例:
#   ./run_capture.sh minimax MiniMax-M2.7 openai sk-xxx
#   ./run_capture.sh glm glm-5.1 anthropic sk-xxx
#
# 输出目录结构:
#   tests/fixtures/llm/v2/{provider}/{model}/{protocol}/{scenario}.json
#   tests/fixtures/llm/v2/{provider}/{model}/{protocol}/{scenario}.txt  (流式)
# ============================================================

set -euo pipefail

PROVIDER="${1:-}"
MODEL="${2:-}"
PROTOCOL="${3:-}"
API_KEY="${4:-}"

if [[ -z "$PROVIDER" || -z "$MODEL" || -z "$PROTOCOL" || -z "$API_KEY" ]]; then
    echo "用法: $0 <provider> <model> <protocol> <api_key>"
    echo "  provider:  minimax | glm | deepseek"
    echo "  protocol:  openai | anthropic"
    echo "  model:     如 MiniMax-M2.7 / glm-5.1 / deepseek-v4-flash"
    echo "  api_key:   API key"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CAPTURE_PY="$SCRIPT_DIR/capture_fixtures.py"
OUTPUT_BASE="$SCRIPT_DIR/../fixtures/llm/v2"

# 清理该 (provider/model/protocol) 组合下的旧文件（如果存在）
TARGET_DIR="$OUTPUT_BASE/$PROVIDER/$MODEL/$PROTOCOL"
if [[ -d "$TARGET_DIR" ]]; then
    echo "[CLEAN] 清理旧文件: $TARGET_DIR"
    rm -f "$TARGET_DIR"/*.json "$TARGET_DIR"/*.txt "$TARGET_DIR"/*-meta.json
fi

echo "=========================================="
echo "Fixture 采集"
echo "  provider: $PROVIDER"
echo "  model:    $MODEL"
echo "  protocol: $PROTOCOL"
echo "  output:   $TARGET_DIR"
echo "=========================================="

# 运行 Python 采集脚本
# Python 脚本内部根据 protocol 确定场景列表，根据 provider+scenario 过滤适用性
python3 "$CAPTURE_PY" \
    --provider "$PROVIDER" \
    --model "$MODEL" \
    --protocol "$PROTOCOL" \
    --api-key "$API_KEY" \
    --output-base "$OUTPUT_BASE"

# 验证：检查输出目录是否有文件
FILE_COUNT=$(find "$TARGET_DIR" -maxdepth 1 \( -name "*.json" -o -name "*.txt" \) 2>/dev/null | wc -l)
echo ""
echo "=========================================="
echo "完成: $FILE_COUNT 个文件输出到 $TARGET_DIR"
echo "=========================================="