#!/usr/bin/env bash
# Capture real GLM Coding Plan API responses as test fixtures
# Models: GLM-5.1, GLM-5-Turbo, GLM-4.7, GLM-4.5-Air
# IMPORTANT: Do NOT commit API keys. Run with env var or stdin input.

set -e

KEY="${GLM_API_KEY?Need GLM_API_KEY env var}"
BASE="https://open.bigmodel.cn/api/coding/paas/v4/chat/completions"
OUTDIR="$(cd "$(dirname "$0")" && pwd)"

mkdir -p "$OUTDIR"

sleep 1

capture() {
  local name="$1"
  local body="$2"
  echo "[GLM] Capturing: $name"
  curl -s -X POST "$BASE" \
    -H "Authorization: Bearer $KEY" \
    -H "Content-Type: application/json" \
    -d "$body" | jq . > "$OUTDIR/$name.json"
  echo "  -> $OUTDIR/$name.json"
  sleep 1
}

# 1. Simple chat, short response (GLM-4.7)
capture "glm-4.7-simple-chat" '{
  "model": "GLM-4.7",
  "messages": [{"role": "user", "content": "Say hello in exactly 3 words"}],
  "temperature": 0.7,
  "max_tokens": 50
}'

# 2. Math question, temperature 0 (GLM-4.7)
capture "glm-4.7-math-temp0" '{
  "model": "GLM-4.7",
  "messages": [{"role": "user", "content": "What is 2+2?"}],
  "temperature": 0,
  "max_tokens": 20
}'

# 3. GLM-5.1 basic
capture "glm-5.1-chat" '{
  "model": "GLM-5.1",
  "messages": [{"role": "user", "content": "Say hi in 3 words"}],
  "temperature": 0,
  "max_tokens": 30
}'

# 4. GLM-5-Turbo basic
capture "glm-5-turbo-chat" '{
  "model": "GLM-5-Turbo",
  "messages": [{"role": "user", "content": "Say hi in 3 words"}],
  "temperature": 0,
  "max_tokens": 30
}'

# 5. GLM-4.5-Air basic
capture "glm-4.5-air-chat" '{
  "model": "GLM-4.5-Air",
  "messages": [{"role": "user", "content": "Say hi in 3 words"}],
  "temperature": 0,
  "max_tokens": 30
}'

# 6. Multi-turn conversation
capture "glm-4.7-multi-turn" '{
  "model": "GLM-4.7",
  "messages": [
    {"role": "user", "content": "My name is Alice"},
    {"role": "assistant", "content": "Hello Alice!"},
    {"role": "user", "content": "What is my name?"}
  ],
  "temperature": 0,
  "max_tokens": 30
}'

# 7. System prompt
capture "glm-4.7-system-prompt" '{
  "model": "GLM-4.7",
  "messages": [
    {"role": "system", "content": "You are a helpful assistant that answers in emojis."},
    {"role": "user", "content": "Say hi"}
  ],
  "temperature": 0.9,
  "max_tokens": 20
}'

# 8. Very short max_tokens (finish_reason=length)
capture "glm-4.7-short-max-tokens" '{
  "model": "GLM-4.7",
  "messages": [{"role": "user", "content": "Tell me a joke"}],
  "temperature": 0,
  "max_tokens": 5
}'

# 9. Long response (to trigger finish_reason=length with larger limit)
capture "glm-4.7-long-response" '{
  "model": "GLM-4.7",
  "messages": [{"role": "user", "content": "Write a paragraph about dogs"}],
  "temperature": 0,
  "max_tokens": 300
}'

# 10. Code generation
capture "glm-4.7-code-generation" '{
  "model": "GLM-4.7",
  "messages": [{"role": "user", "content": "Write a hello world in Python"}],
  "temperature": 0,
  "max_tokens": 100
}'

# 11. Unicode content (Chinese)
capture "glm-4.7-unicode-chat" '{
  "model": "GLM-4.7",
  "messages": [{"role": "user", "content": "用三个词形容春天"}],
  "temperature": 0,
  "max_tokens": 30
}'

# 12. Reasoning / math heavy
capture "glm-5.1-reasoning" '{
  "model": "GLM-5.1",
  "messages": [{"role": "user", "content": "What is 17 * 23? Show your work."}],
  "temperature": 0,
  "max_tokens": 200
}'

# 13. Long conversation history
capture "glm-4.7-long-history" '{
  "model": "GLM-4.7",
  "messages": [
    {"role": "user", "content": "I like apples"},
    {"role": "assistant", "content": "Apples are great! Red or green?"},
    {"role": "user", "content": "Red please"},
    {"role": "assistant", "content": "Red apples are juicy and sweet!"},
    {"role": "user", "content": "What did I say I like?"}
  ],
  "temperature": 0,
  "max_tokens": 30
}'

# 14. Error: invalid model
capture "glm-error-invalid-model" '{
  "model": "invalid-model-xyz",
  "messages": [{"role": "user", "content": "hi"}],
  "max_tokens": 10
}'

# 15. Error: empty messages
capture "glm-error-empty-messages" '{
  "model": "GLM-4.7",
  "messages": [],
  "max_tokens": 10
}'

# 16. Temperature 1.0
capture "glm-4.7-temp-1.0" '{
  "model": "GLM-4.7",
  "messages": [{"role": "user", "content": "Give me a random word"}],
  "temperature": 1.0,
  "max_tokens": 10
}'

# --- Streaming ---
echo "[GLM] Capturing: streaming-glm-4.7"
curl -s -X POST "$BASE" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "GLM-4.7",
    "messages": [{"role": "user", "content": "Count to 3"}],
    "temperature": 0,
    "max_tokens": 30,
    "stream": true
  }' > "$OUTDIR/streaming-glm-4.7.txt"
echo "  -> $OUTDIR/streaming-glm-4.7.txt"
sleep 1

echo "[GLM] Capturing: streaming-glm-5.1"
curl -s -X POST "$BASE" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "GLM-5.1",
    "messages": [{"role": "user", "content": "What is 2+2?"}],
    "temperature": 0,
    "max_tokens": 50,
    "stream": true
  }' > "$OUTDIR/streaming-glm-5.1.txt"
echo "  -> $OUTDIR/streaming-glm-5.1.txt"
sleep 1

# --- Summary ---
echo ""
echo "=== Capture complete ==="
ls -la "$OUTDIR/" | awk '{print $9, $5}' | while read -r f s; do
  echo "  $s  $(basename "$f")"
done