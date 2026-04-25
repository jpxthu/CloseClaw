#!/usr/bin/env bash
# Capture real MiniMax API responses as test fixtures for closeclaw/src/llm/
# Usage: MINIMAX_API_KEY=your_key ./capture-minimax-fixtures.sh

set -e

KEY="${MINIMAX_API_KEY?Need MINIMAX_API_KEY env var}"
BASE_URL="https://api.minimaxi.com/v1/text/chatcompletion_v2"
OUTDIR="$(dirname "$0")/../fixtures/llm/minimax"
TEMP="/tmp/minimax-fixture-$$.json"

mkdir -p "$OUTDIR"

capture() {
  local name="$1"
  local body="$2"
  echo "Capturing: $name"
  curl -s -X POST "$BASE_URL" \
    -H "Authorization: Bearer $KEY" \
    -H "Content-Type: application/json" \
    -d "$body" | jq . > "$OUTDIR/$name.json"
  echo "  -> $OUTDIR/$name.json"
}

# 1. Simple chat, short response
capture "simple-chat" '{
  "model": "MiniMax-M2.5",
  "messages": [{"role": "user", "content": "Say hello in exactly 3 words"}],
  "temperature": 0.7,
  "max_tokens": 50
}'

# 2. Math question, temperature 0
capture "math-temp0" '{
  "model": "MiniMax-M2.5",
  "messages": [{"role": "user", "content": "What is 2+2?"}],
  "temperature": 0,
  "max_tokens": 20
}'

# 3. MiniMax-M2.7
capture "m2.7-chat" '{
  "model": "MiniMax-M2.7",
  "messages": [{"role": "user", "content": "Say hello in 3 words"}],
  "temperature": 0,
  "max_tokens": 30
}'

# 4. Multi-turn conversation (history)
capture "multi-turn" '{
  "model": "MiniMax-M2.5",
  "messages": [
    {"role": "user", "content": "My name is Alice"},
    {"role": "assistant", "content": "Hello Alice!"},
    {"role": "user", "content": "What is my name?"}
  ],
  "temperature": 0,
  "max_tokens": 30
}'

# 5. System prompt
capture "system-prompt" '{
  "model": "MiniMax-M2.5",
  "messages": [
    {"role": "system", "content": "You are a helpful assistant that answers in emojis."},
    {"role": "user", "content": "Say hi"}
  ],
  "temperature": 0.9,
  "max_tokens": 20
}'

# 6. Error: invalid model
capture "error-invalid-model" '{
  "model": "invalid-model-xyz",
  "messages": [{"role": "user", "content": "hi"}],
  "max_tokens": 10
}'

# 7. Error: empty messages
capture "error-empty-messages" '{
  "model": "MiniMax-M2.5",
  "messages": [],
  "max_tokens": 10
}'

# 8. Usage endpoint
echo "Capturing: usage-coding-plan"
curl -s -X GET "https://api.minimaxi.com/v1/api/openplatform/coding_plan/remains" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -H "MM-API-Source: OpenClaw" | jq . > "$OUTDIR/usage-coding-plan.json"
echo "  -> $OUTDIR/usage-coding-plan.json"

# 9. Streaming (capture first few chunks)
echo "Capturing: streaming"
curl -s -X POST "$BASE_URL" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "MiniMax-M2.5",
    "messages": [{"role": "user", "content": "Count to 3"}],
    "temperature": 0,
    "max_tokens": 30,
    "stream": true
  }' > "$OUTDIR/streaming.txt"
echo "  -> $OUTDIR/streaming.txt"

# Summary
echo ""
echo "=== Capture complete ==="
ls -la "$OUTDIR/"
