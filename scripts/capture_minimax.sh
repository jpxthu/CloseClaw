#!/usr/bin/env bash
#
# MiniMax 场景批量采集
# 用法: bash scripts/capture_minimax.sh [MODEL]
# 默认模型: MiniMax-M2.7
#
set -euo pipefail

MODEL="${1:-MiniMax-M2.7}"
OUTPUT_BASE="tests/fixtures/llm/v2"
PROVIDER="minimax"
PROJECT_DIR="$HOME/code/closeclaw-test"

if [[ -z "${LLM_API_KEY:-}" ]]; then
    echo "Error: LLM_API_KEY environment variable not set" >&2
    exit 1
fi

cd "$PROJECT_DIR"

# 场景列表
SCENARIOS=(
    "simple"
    "streaming"
    "multi-turn"
    "cache"
    "minimax-reasoning-split"
    "tool-use"
    "error-auth"
    "error-model"
    "error-empty"
    "anthropic-simple"
    "anthropic-thinking"
    "anthropic-streaming"
    "anthropic-error-auth"
    "anthropic-error-empty"
)

echo "========================================"
echo "MiniMax Fixture Capture"
echo "Model: $MODEL"
echo "Output: $OUTPUT_BASE/$PROVIDER/"
echo "========================================"
echo ""

for SCENARIO in "${SCENARIOS[@]}"; do
    echo "[$PROVIDER] $MODEL × $SCENARIO"
    python3 "$PROJECT_DIR/tests/fixtures/llm/v2/capture_fixtures.py" \
        --provider "$PROVIDER" \
        --model "$MODEL" \
        --scenario "$SCENARIO" \
        --api-key "$LLM_API_KEY" \
        --output-base "$OUTPUT_BASE" \
        || echo "  ⚠️  failed, continuing..."
    echo ""
done

echo "========================================"
echo "Done. Outputs in $OUTPUT_BASE/$PROVIDER/"
echo "========================================"
