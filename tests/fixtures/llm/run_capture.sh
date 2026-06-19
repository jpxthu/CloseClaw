#!/usr/bin/env bash
# ============================================================
# LLM Fixture 采集入口 — Phase 1 重构
#
# 用法:
#   ./run_capture.sh <provider> <model> <protocol> <api_key> [scenario_type]
#
# 示例:
#   ./run_capture.sh minimax MiniMax-M2.7 openai sk-xxx
#   ./run_capture.sh glm glm-5.1 anthropic sk-xxx
#   ./run_capture.sh deepseek deepseek-v4-pro openai sk-xxx all
#   ./run_capture.sh deepseek deepseek-v4-pro openai sk-xxx non-chat
#
# scenario_type: chat | non-chat | all (默认 all)
#
# 输出目录结构:
#   chat 场景:    tests/fixtures/llm/v2/{provider}/{model}/{protocol}/{scenario}.json
#   非 chat 场景: tests/fixtures/llm/v2/{provider}/provider/{scenario}.json
# ============================================================

set -euo pipefail

PROVIDER="${1:-}"
MODEL="${2:-}"
PROTOCOL="${3:-}"
API_KEY="${4:-}"
SCENARIO_TYPE="${5:-all}"

if [[ -z "$PROVIDER" || -z "$MODEL" || -z "$PROTOCOL" || -z "$API_KEY" ]]; then
    echo "用法: $0 <provider> <model> <protocol> <api_key> [scenario_type]"
    echo "  provider:       minimax | glm | deepseek"
    echo "  protocol:       openai | anthropic"
    echo "  model:          如 MiniMax-M2.7 / glm-5.1 / deepseek-v4-flash"
    echo "  api_key:        API key"
    echo "  scenario_type:  chat | non-chat | all (默认 all)"
    exit 1
fi

if [[ "$SCENARIO_TYPE" != "chat" && "$SCENARIO_TYPE" != "non-chat" && "$SCENARIO_TYPE" != "all" ]]; then
    echo "Error: scenario_type 必须是 chat, non-chat 或 all"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CAPTURE_PY="$SCRIPT_DIR/capture_fixtures.py"
OUTPUT_BASE="$SCRIPT_DIR"

# 清理该 (provider/model/protocol) 组合下的旧文件（如果存在）
if [[ "$SCENARIO_TYPE" == "chat" || "$SCENARIO_TYPE" == "all" ]]; then
    TARGET_DIR="$OUTPUT_BASE/$PROVIDER/$MODEL/$PROTOCOL"
    if [[ -d "$TARGET_DIR" ]]; then
        echo "[CLEAN] 清理旧文件: $TARGET_DIR"
        rm -f "$TARGET_DIR"/*.json "$TARGET_DIR"/*.txt "$TARGET_DIR"/*-meta.json
    fi
fi

echo "=========================================="
echo "Fixture 采集"
echo "  provider:       $PROVIDER"
echo "  model:          $MODEL"
echo "  protocol:       $PROTOCOL"
echo "  scenario_type:  $SCENARIO_TYPE"
echo "=========================================="

# 运行 Python 采集脚本
python3 "$CAPTURE_PY" \
    --provider "$PROVIDER" \
    --model "$MODEL" \
    --protocol "$PROTOCOL" \
    --api-key "$API_KEY" \
    --output-base "$OUTPUT_BASE" \
    --scenario-type "$SCENARIO_TYPE"

# 验证：检查输出目录
echo ""
echo "=========================================="
if [[ "$SCENARIO_TYPE" == "chat" || "$SCENARIO_TYPE" == "all" ]]; then
    TARGET_DIR="$OUTPUT_BASE/$PROVIDER/$MODEL/$PROTOCOL"
    FILE_COUNT=$(find "$TARGET_DIR" -maxdepth 1 \( -name "*.json" -o -name "*.txt" \) 2>/dev/null | wc -l)
    echo "Chat 场景: $FILE_COUNT 个文件输出到 $TARGET_DIR"
fi
if [[ "$SCENARIO_TYPE" == "non-chat" || "$SCENARIO_TYPE" == "all" ]]; then
    PROVIDER_DIR="$OUTPUT_BASE/$PROVIDER/provider"
    if [[ -d "$PROVIDER_DIR" ]]; then
        FILE_COUNT=$(find "$PROVIDER_DIR" -maxdepth 1 -name "*.json" 2>/dev/null | wc -l)
        echo "非 Chat 场景: $FILE_COUNT 个文件输出到 $PROVIDER_DIR"
    else
        echo "非 Chat 场景: 无输出目录"
    fi
fi
echo "=========================================="
