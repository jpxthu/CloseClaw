# MiniMax LLM API Fixtures

从真实 MiniMax API 捕获的请求/响应样本，用于 closeclaw LLM 模块的测试开发。

## 文件清单

### OpenAI 兼容接口 (`/v1/text/chatcompletion_v2`)

| 文件 | 说明 |
|------|------|
| `simple-chat.json` | 常规对话，temp=0.7 |
| `math-temp0.json` | 数学问题，temp=0 |
| `m2.7-chat.json` | MiniMax-M2.7 模型 |
| `m2.5-highspeed-chat.json` | M2.5-highspeed |
| `m2.7-highspeed-chat.json` | M2.7-highspeed |
| `m2-her-chat.json` | M2-her（角色扮演模型） |
| `multi-turn.json` | 多轮对话（含历史） |
| `long-history.json` | 5轮长历史对话 |
| `system-prompt.json` | 带 system prompt |
| `code-generation.json` | 代码生成 |
| `reasoning-heavy.json` | 重推理任务（数学） |
| `unicode-chat.json` | 中文 prompt |
| `long-prompt.json` | 长用户 prompt |
| `long-response.json` | 长回答（触发 finish_reason=length） |
| `short-max-tokens.json` | 极短 max_tokens（5） |
| `temp-1.0.json` | temperature=1.0 |
| `streaming.txt` | 流式响应（非 Anthropic 端点） |
| `streaming-m2.7.txt` | M2.7 流式响应 |
| `error-invalid-model.json` | 错误：无效模型 |
| `error-empty-messages.json` | 错误：空消息列表 |
| `error-missing-model.json` | 错误：缺少 model 字段 |
| `error-auth.json` | 错误：认证失败（1004） |

### Anthropic 兼容接口 (`/anthropic/v1/messages`)

| 文件 | 说明 |
|------|------|
| `anthropic-basic.json` | 基础对话 |
| `anthropic-with-system.json` | 带 system prompt |
| `anthropic-thinking-block.json` | 含 thinking + text 双块 |
| `anthropic-m2-her.json` | M2-her 模型 |
| `anthropic-streaming.txt` | Anthropic 流式响应 |
| `anthropic-error-invalid-model.json` | 错误：无效模型 |
| `anthropic-error-auth.json` | 错误：认证失败 |

### 用量接口

| 文件 | 说明 |
|------|------|
| `usage-coding-plan.json` | `/openplatform/coding_plan/remains` |

---

## 重要发现

### OpenAI 兼容接口（closeclaw 使用的）

```
Response.message.content     → 永远为空字符串
Response.message.reasoning_content → 实际回答内容（纯文本）
Response.usage.completion_tokens → 包含 reasoning token，非纯回答 token
Response.choices[].finish_reason  → "length" / "stop"
```

**所有 OpenAI 兼容接口的响应，`message.content` 都是空的**，实际内容在 `message.reasoning_content`。

### Anthropic 兼容接口

```
Response.content → 数组，可能包含：
  {"type": "thinking", "thinking": "...", "signature": "..."}  ← 推理块
  {"type": "text",     "text": "..."}                        ← 最终回答块
Response.stop_reason → "end_turn" / "max_tokens"
Response.usage.input_tokens / output_tokens（而非 prompt/completion）
```

### M2-her 模型

使用 OpenAI 兼容接口格式，但：
- `message.content` 有实际内容（角色扮演回答）
- `finish_reason` = "stop"（不是 "length"）
- 无 `reasoning_content` 字段

### 错误码

| status_code | 含义 |
|------------|------|
| 0 | 成功 |
| 1004 | 认证失败 |
| 2013 | 无效模型 / 参数错误 |

---

## 使用方式

```rust
// 在测试中引用 fixture
let fixture = include_str!("../../../tests/fixtures/llm/minimax/simple-chat.json");
let resp: MiniMaxResponse = serde_json::from_str(fixture).unwrap();
```

## 更新方式

```bash
export MINIMAX_API_KEY=your_key
bash tests/fixtures/llm/minimax/capture-minimax-fixtures.sh
```
