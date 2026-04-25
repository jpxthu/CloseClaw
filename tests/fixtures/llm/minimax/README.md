# MiniMax LLM API Fixtures

从真实 MiniMax API 捕获的请求/响应样本，用于 closeclaw LLM 模块的测试开发。

## 文件说明

| 文件 | 说明 |
|------|------|
| `simple-chat.json` | 常规对话，temp=0.7，max_tokens=50 |
| `math-temp0.json` | 数学问题，temp=0，max_tokens=20 |
| `m2.7-chat.json` | MiniMax-M2.7 模型 |
| `multi-turn.json` | 多轮对话（含对话历史） |
| `system-prompt.json` | 带 system prompt，temp=0.9 |
| `error-invalid-model.json` | 错误响应：无效模型名 |
| `error-empty-messages.json` | 错误响应：空消息列表 |
| `usage-coding-plan.json` | 用量查询接口 `/openplatform/coding_plan/remains` |
| `streaming.txt` | 流式响应原始 SSE 数据 |
| `capture-minimax-fixtures.sh` | 捕获脚本（需设置 `MINIMAX_API_KEY`） |

## 重要发现

MiniMax-M2/M2.1/M2.5/M2.7 API 的响应结构与 OpenAI 兼容格式有以下差异：

1. **`message.content` 永远为空**，实际回答在 `message.reasoning_content` 字段
2. **`completion_tokens` 包含 reasoning token**，不是纯最终回答的 token 数
3. **`finish_reason` 可能总是返回 `"length"`**（即使回答完整），需注意
4. 流式响应中，`delta.reasoning_content` 携带增量内容，`delta.content` 为空

## 使用方式

```rust
// 在测试中引用 fixture
let fixture = include_str!("../../../tests/fixtures/llm/minimax/simple-chat.json");
let resp: MiniMaxResponse = serde_json::from_str(fixture).unwrap();
```

## 更新方式

```bash
MINIMAX_API_KEY=your_key ./capture-minimax-fixtures.sh
```
