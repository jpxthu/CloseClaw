#!/usr/bin/env bash
# Capture real MiniMax API responses as test fixtures
# Usage: MINIMAX_API_KEY=your_key ./capture-minimax-fixtures.sh

set -e

KEY="${MINIMAX_API_KEY?Need MINIMAX_API_KEY env var}"
OPENAI_BASE="https://api.minimaxi.com/v1/text/chatcompletion_v2"
ANTHROPIC_BASE="https://api.minimaxi.com/anthropic/v1/messages"
OUTDIR="$(cd "$(dirname "$0")" && pwd)"

mkdir -p "$OUTDIR"

sleep 1

# --- OpenAI 兼容接口 ---

capture() {
  local name="$1"
  local body="$2"
  echo "[OpenAI] Capturing: $name"
  curl -s -X POST "$OPENAI_BASE" \
    -H "Authorization: Bearer $KEY" \
    -H "Content-Type: application/json" \
    -d "$body" | jq . > "$OUTDIR/$name.json"
  echo "  -> $OUTDIR/$name.json"
  sleep 1
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

# 8. Temperature 1.0 (recommended)
capture "temp-1.0" '{
  "model": "MiniMax-M2.5",
  "messages": [{"role": "user", "content": "Give me a random word"}],
  "temperature": 1.0,
  "max_tokens": 10
}'

# 9. Very short max_tokens (finish_reason=stop test)
capture "short-max-tokens" '{
  "model": "MiniMax-M2.5",
  "messages": [{"role": "user", "content": "Tell me a joke"}],
  "temperature": 0,
  "max_tokens": 5
}'

# 10. Long response (to trigger finish_reason=length)
capture "long-response" '{
  "model": "MiniMax-M2.5",
  "messages": [{"role": "user", "content": "Write a paragraph about dogs"}],
  "temperature": 0,
  "max_tokens": 300
}'

# 11. M2.5-highspeed
capture "m2.5-highspeed-chat" '{
  "model": "MiniMax-M2.5-highspeed",
  "messages": [{"role": "user", "content": "Say hi in one word"}],
  "temperature": 0,
  "max_tokens": 10
}'

# 12. M2.7-highspeed
capture "m2.7-highspeed-chat" '{
  "model": "MiniMax-M2.7-highspeed",
  "messages": [{"role": "user", "content": "Say hi in one word"}],
  "temperature": 0,
  "max_tokens": 10
}'

# 13. M2-her (role play / character chat)
capture "m2-her-chat" '{
  "model": "M2-her",
  "messages": [
    {"role": "system", "content": "You are a pirate captain."},
    {"role": "user", "content": "Where be we sailing today?"}
  ],
  "temperature": 0.8,
  "max_tokens": 50
}'

# 14. Code generation
capture "code-generation" '{
  "model": "MiniMax-M2.5",
  "messages": [{"role": "user", "content": "Write a hello world in Python"}],
  "temperature": 0,
  "max_tokens": 100
}'

# 15. Math / reasoning heavy
capture "reasoning-heavy" '{
  "model": "MiniMax-M2.7",
  "messages": [{"role": "user", "content": "What is 17 * 23? Show your work."}],
  "temperature": 0,
  "max_tokens": 200
}'

# 16. Streaming
echo "[OpenAI] Capturing: streaming"
curl -s -X POST "$OPENAI_BASE" \
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
sleep 1

# 17. Streaming with M2.7
echo "[OpenAI] Capturing: streaming-m2.7"
curl -s -X POST "$OPENAI_BASE" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "MiniMax-M2.7",
    "messages": [{"role": "user", "content": "What is 2+2?"}],
    "temperature": 0,
    "max_tokens": 50,
    "stream": true
  }' > "$OUTDIR/streaming-m2.7.txt"
echo "  -> $OUTDIR/streaming-m2.7.txt"
sleep 1

# 18. Long conversation history (many turns)
capture "long-history" '{
  "model": "MiniMax-M2.5",
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

# 19. Unicode content (Chinese)
capture "unicode-chat" '{
  "model": "MiniMax-M2.5",
  "messages": [{"role": "user", "content": "用三个词形容春天"}],
  "temperature": 0,
  "max_tokens": 30
}'

# 20. Error: auth failure (wrong key prefix)
echo "[OpenAI] Capturing: error-auth"
curl -s -X POST "$OPENAI_BASE" \
  -H "Authorization: Bearer invalid-key-xyz" \
  -H "Content-Type: application/json" \
  -d '{"model": "MiniMax-M2.5", "messages": [{"role": "user", "content": "hi"}], "max_tokens": 10}' \
  | jq . > "$OUTDIR/error-auth.json"
echo "  -> $OUTDIR/error-auth.json"
sleep 1

# 21. Error: missing required field
capture "error-missing-model" '{
  "messages": [{"role": "user", "content": "hi"}],
  "max_tokens": 10
}'

# 22. Very long user prompt (to test prompt_tokens handling)
capture "long-prompt" '{
  "model": "MiniMax-M2.5",
  "messages": [{"role": "user", "content": "Tell me a story. Once upon a time in a land far away there lived a brave knight who fought dragons and saved princesses. The end."}],
  "temperature": 0,
  "max_tokens": 50
}'

# --- Anthropic 兼容接口 ---

capture_anthropic() {
  local name="$1"
  local body="$2"
  echo "[Anthropic] Capturing: $name"
  curl -s -X POST "$ANTHROPIC_BASE" \
    -H "Authorization: Bearer $KEY" \
    -H "Content-Type: application/json" \
    -H "anthropic-version: 2023-06-01" \
    -d "$body" | jq . > "$OUTDIR/anthropic-$name.json"
  echo "  -> $OUTDIR/anthropic-$name.json"
  sleep 1
}

# 23. Anthropic: basic chat
capture_anthropic "basic" '{
  "model": "MiniMax-M2.5",
  "max_tokens": 50,
  "messages": [{"role": "user", "content": "Say hello in 3 words"}]
}'

# 24. Anthropic: with system prompt
capture_anthropic "with-system" '{
  "model": "MiniMax-M2.5",
  "max_tokens": 50,
  "system": "You are a pirate.",
  "messages": [{"role": "user", "content": "Say hello"}]
}'

# 25. Anthropic: thinking block present
capture_anthropic "thinking-block" '{
  "model": "MiniMax-M2.7",
  "max_tokens": 100,
  "messages": [{"role": "user", "content": "What is 17 * 23?"}]
}'

# 26. Anthropic: streaming
echo "[Anthropic] Capturing: streaming"
curl -s -X POST "$ANTHROPIC_BASE" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "MiniMax-M2.5",
    "max_tokens": 50,
    "messages": [{"role": "user", "content": "Count to 3"}],
    "stream": true
  }' > "$OUTDIR/anthropic-streaming.txt"
echo "  -> $OUTDIR/anthropic-streaming.txt"
sleep 1

# 27. Anthropic: M2-her
capture_anthropic "m2-her" '{
  "model": "M2-her",
  "max_tokens": 50,
  "system": "You are a wizard.",
  "messages": [{"role": "user", "content": "Cast a spell"}]
}'

# 28. Anthropic: error - invalid model
echo "[Anthropic] Capturing: error-invalid-model"
curl -s -X POST "$ANTHROPIC_BASE" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model": "invalid-model", "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}]}' \
  | jq . > "$OUTDIR/anthropic-error-invalid-model.json"
echo "  -> $OUTDIR/anthropic-error-invalid-model.json"
sleep 1

# 29. Anthropic: error - auth
echo "[Anthropic] Capturing: error-auth"
curl -s -X POST "$ANTHROPIC_BASE" \
  -H "Authorization: Bearer wrong-key" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model": "MiniMax-M2.5", "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}]}' \
  | jq . > "$OUTDIR/anthropic-error-auth.json"
sleep 1

# --- Usage / billing endpoints ---

echo "Capturing: usage-coding-plan"
curl -s -X GET "https://api.minimaxi.com/v1/api/openplatform/coding_plan/remains" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -H "MM-API-Source: OpenClaw" | jq . > "$OUTDIR/usage-coding-plan.json"
echo "  -> $OUTDIR/usage-coding-plan.json"

# --- Summary ---
echo ""
echo "=== Capture complete ==="
ls -la "$OUTDIR/" | awk '{print $9, $5}' | while read -r f s; do
  echo "  $s  $(basename "$f")"
done
