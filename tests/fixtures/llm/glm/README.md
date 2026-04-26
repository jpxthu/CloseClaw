# GLM Coding Plan API Fixtures

Real API responses captured from `https://open.bigmodel.cn/api/coding/paas/v4/chat/completions`.

**Supported models**: GLM-5.1, GLM-5-Turbo, GLM-4.7, GLM-4.5-Air

**Important observations** (differences from OpenAI/MiniMax):

| Aspect | GLM Behavior |
|--------|--------------|
| Reasoning models | Uses `reasoning_content` field (like MiniMax), NOT `content`. `content` is empty string for reasoning models |
| finish_reason=length | Triggers when `max_tokens` limit hit — still fills `reasoning_content` |
| Error format | `{ "error": { "code": "1211", "message": "..." } }` — top-level `error` key, not nested |
| Model name in response | May differ in casing from request (e.g., `"GLM-4.7"` → `"glm-5.1"`) |
| Reasoning tokens | Tracked in `usage.completion_tokens_details.reasoning_tokens` |
| Cached tokens | Tracked in `usage.prompt_tokens_details.cached_tokens` |
| Streaming delta | Uses `reasoning_content` in `delta`, NOT `content` |
| Streaming format | SSE `data: ` prefix per line |

## Chat Completions Fixtures

| File | Model | Description |
|------|-------|-------------|
| `glm-5.1-chat.json` | GLM-5.1 | Basic chat, short response |
| `glm-5.1-multi-turn.json` | GLM-5.1 | 3-turn conversation |
| `glm-5.1-system-prompt.json` | GLM-5.1 | With system prompt, emoji style |
| `glm-5.1-short-max-tokens.json` | GLM-5.1 | `finish_reason=length`, max_tokens=5 |
| `glm-5.1-code-generation.json` | GLM-5.1 | Python hello world |
| `glm-5.1-unicode-chat.json` | GLM-5.1 | Chinese content |
| `glm-5.1-temp-1.0.json` | GLM-5.1 | Temperature 1.0 |
| `glm-5.1-long-history.json` | GLM-5.1 | 5-turn conversation history |
| `glm-5.1-reasoning.json` | GLM-5.1 | Reasoning-heavy math question |
| `glm-5-turbo-chat.json` | GLM-5-Turbo | Basic chat |
| `glm-4.7-simple-chat.json` | GLM-4.7 | Basic chat, temperature 0.7 |
| `glm-4.7-math-temp0.json` | GLM-4.7 | Math answer, temperature 0 |
| `glm-4.7-short-max-tokens.json` | GLM-4.7 | `finish_reason=length`, max_tokens=5 |
| `glm-4.7-long-response.json` | GLM-4.7 | Full paragraph, 300 tokens |
| `glm-4.7-multi-turn.json` | GLM-4.7 | 3-turn conversation |
| `glm-4.7-system-prompt.json` | GLM-4.7 | With system prompt |
| `glm-4.7-code-generation.json` | GLM-4.7 | Python hello world |
| `glm-4.7-unicode-chat.json` | GLM-4.7 | Chinese content |
| `glm-4.7-long-history.json` | GLM-4.7 | 5-turn conversation history |
| `glm-4.7-temp-1.0.json` | GLM-4.7 | Temperature 1.0 |
| `glm-4.5-air-chat.json` | GLM-4.5-Air | Basic chat |
| `glm-error-invalid-model.json` | — | Error: invalid model (code 1211) |
| `glm-error-empty-messages.json` | — | Error: empty messages (code 1214) |

## Streaming Fixtures

| File | Model | Description |
|------|-------|-------------|
| `streaming-glm-4.7.txt` | GLM-4.7 | SSE streaming, "Count to 3" |
| `streaming-glm-5.1.txt` | GLM-5.1 | SSE streaming, "What is 2+2?" |
| `streaming-glm-5.1-v2.txt` | GLM-5.1 | SSE streaming, another sample |

## Usage Fixtures

| File | Endpoint | Description |
|------|----------|-------------|
| `usage-glm-coding-plan.json` | `open.bigmodel.cn/api/monitor/usage/quota/limit` | CN endpoint, Pro plan |
| `usage-glm-global.json` | `api.z.ai/api/monitor/usage/quota/limit` | Global endpoint, same data |

**Usage response key fields**:
- `level`: plan tier (`pro`, `lite`, `max`)
- `TOKENS_LIMIT` `unit=3`: 5-hour window, `percentage` shows usage
- `TOKENS_LIMIT` `unit=6`: weekly/monthly window
- `TIME_LIMIT` `unit=5`: MCP quota (search, web-reader, zread)

## Capture Script

See `capture-glm-fixtures.sh` — run with `GLM_API_KEY=your_key ./capture-glm-fixtures.sh`.

**Note**: Coding Plan does not support Anthropic-compatible API endpoint. Only the OpenAI-compatible `/coding/paas/v4/chat/completions` endpoint is available.

## MCP Fixtures

MCP servers use SSE response format (`id:` + `event:message` + `data:` lines).

**Endpoint**: `https://open.bigmodel.cn/api/mcp/<service>/mcp`

| File | MCP Server | Content |
|------|-----------|---------|
| `mcp-web-search-prime-list.json` | Web Search | `tools/list` — `web_search_prime` tool |
| `mcp-web-reader-list.json` | Web Reader | `tools/list` — `webReader` tool |
| `mcp-zread-list.json` | ZRead | `tools/list` — `search_doc`, `read_file`, `get_repo_structure` |

**MCP handshake** (web_search_prime):
- `initialize` → ✅ Returns protocol version, capabilities, server info (2024-11-05)
- `tools/list` → ✅ Works
- `tools/call` → ❌ Auth fails (`MCP error -401: Api key not found`)
  - The Coding Plan API key works for chat completions but NOT for MCP endpoints
  - MCP endpoints appear to require a different key format or separate MCP access
  - `zread` also returns `MCP error -500` (server-side error)
  - These MCP tools are designed to be called from Claude Code/Cline/etc., not via raw curl

**Note**: Vision MCP (`@z_ai/mcp-server`) is a local npm package, not a remote HTTP endpoint — requires Node.js + MCP client to use.

## Not Tested (Coding Plan limitations)

- Function calling / tool use (MCP tools are the mechanism, not direct API)
- Vision / image input (via Vision MCP local package)
- Audio input
- File attachment