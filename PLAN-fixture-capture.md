# LLM Provider Fixture 采集计划

> 每轮对话前先读此文件：路径 `workspace/PLAN-fixture-capture.md`

## 背景

需要为 CloseClaw 所有 v1 目标 provider（MiniMax / GLM / DeepSeek）采集真实 API 响应 fixture，覆盖多种场景，作为后续功能开发和测试的基准数据。现有 `tests/fixtures/llm/` 下的 fixture 已被 UT 依赖，无法直接覆盖，因此新开 `tests/fixtures/llm/v2/` 存放新采集的数据。

**重要**：写脚本之前，先通过 provider 官方文档做迭代深挖，汇总所有相关 API 信息，输出到 `tests/fixtures/llm/v2/docs/` 下，再基于这些文档编写 capture 脚本。

## 目的

1. 通过官方文档迭代深挖，整理各 provider 的完整 API 信息
2. 验证各 provider 对 reasoning/thinking 相关参数的支持情况
3. 在 `tests/fixtures/llm/v2/` 下建立新的 fixture 集合
4. capture 脚本和 fixture 数据都放在 `v2/` 下，文件名标明 `openai` / `anthropic` 协议
5. 为 Reasoning Level 实现提供实测依据

---

## 当前 PR

Branch: `feat/llm-fixture-capture-scripts` | PR: https://github.com/jpxthu/CloseClaw/pull/198

当前进度：
- Phase 0 文档深挖 ✅（MiniMax ✅ / GLM ✅ / DeepSeek 待 owner 提供链接）
- Phase 1 脚本框架 ✅（含 `--all` 批量模式）
- Phase 2 MiniMax 采集 ✅（OpenAI + Anthropic）
- Phase 3 GLM 采集 ✅（OpenAI 21场景 × 9模型 + Anthropic 12场景 × 5模型）
- Phase 4 DeepSeek 采集 ❌（待 owner 提供文档链接）
- Phase 5 结果分析 ❌（待 Phase 4 完成）
- Phase 6 README 指引 ❌（待开始）

已采集 fixture 总数：
- `minimax/`：OpenAI + Anthropic 完整场景
- `glm/`：OpenAI（~180文件）+ Anthropic（69文件）

---

## Phase 0：文档深挖（先于脚本）

Owner 将分三次提供各 provider 的文档链接，braino 使用 `iterative-deep-dive` skill 对每个 provider 的相关章节进行多轮深挖，直到收敛。

### 输出位置

`~/code/closeclaw-test/tests/fixtures/llm/v2/docs/`

```
docs/
├── minimax-api-summary.md
├── glm-api-summary.md
└── deepseek-api-summary.md
```

> 代码库路径：`~/code/closeclaw-test/`

### 深挖关注维度（每个 provider 都要覆盖）

| 维度 | 关注内容 |
|------|----------|
| **认证与请求** | API key 格式、base_url、required headers、OpenAI-compatible 还是独立 endpoint |
| **Chat Completions 请求** | endpoint URL、method、必填字段、可选字段、model 名称格式 |
| **Reasoning/Thinking 参数** | 支持的参数名（thinking_budget / reasoning_effort / reasoning_split 等）、生效条件、默认值、上限 |
| **响应格式（OpenAI 兼容）** | choices[].message 结构、usage（prompt_tokens / completion_tokens / total_tokens）、prompt_tokens_details.cached_tokens |
| **响应格式（Anthropic 兼容）** | content[].type=text/thinking、cache_control、usage.block_usage |
| **Streaming 响应** | SSE 格式、chunk 结构、finish_reason、role 字段出现时机 |
| **Error 响应** | 错误格式、status code、error.code / error.type 字段 |
| **Cache 机制** | 主动缓存 vs 被动缓存、cache 相关字段名（如 prompt_cache_hit_tokens / cache_read_input_tokens） |
| **工具调用（Tool Use）** | `tools` 参数格式、`tool_call` 响应格式、`tool_result`、`thinking` 与工具调用关系、Streaming 下的 tool chunk 格式 |
| **多轮对话** | 上下文记忆方式、session 处理方式、最大 history 长度 |
| **厂商特有字段** | 各 provider 私有字段（如 MiniMax 的 extra.body 字段、DeepSeek 的 reasoning_content 等） |
| **具体文档 URL** | 每个结论的来源章节 URL，便于追溯 |

### 深挖流程

1. **Phase 0.1**：Owner 提供 MiniMax 文档链接 → braino 深挖 → 输出 `minimax-api-summary.md`
2. **Phase 0.2**：Owner 提供 GLM 文档链接 → braino 深挖 → 输出 `glm-api-summary.md`
3. **Phase 0.3**：Owner 提供 DeepSeek 文档链接 → braino 深挖 → 输出 `deepseek-api-summary.md`

每个 phase 内部按 `iterative-deep-dive` skill 工作流：重点深挖 → 全量验证 → 整理收敛。

---

## Phase 1：脚本框架

- 文件：`~/code/closeclaw-test/tests/fixtures/llm/v2/capture_fixtures.py`
- 内容：协议抽象 + 场景定义 + 输出目录结构
- 输出目录：`~/code/closeclaw-test/tests/fixtures/llm/v2/{provider}/{protocol}/`
- 文件命名：统一标注 `openai` 或 `anthropic`，如 `minimax-2.7-openai-simple.json` / `minimax-2.7-anthropic-simple.json`
- 场景覆盖：基于深挖文档，确认所有相关 API 和场景都被脚本覆盖
- 状态：待 Phase 0 完成

### 场景定义（基于深挖文档确认）

| 场景 | 说明 |
|------|------|
| `simple` | 基础 single-turn 对话 |
| `reasoning` | 带 reasoning/thinking 参数的请求，验证 content 是否含标签或独立字段 |
| `streaming` | SSE stream 响应，验证 chunk 格式 |
| `cache` | 触发 cache 命中的请求，验证 cache 相关字段 |
| `multi-turn` | 多轮对话，验证历史 context 处理方式 |
| `error-auth` | 错误 API key，验证 error 格式 |
| `error-model` | 无效 model name，验证 error 格式 |
| `error-empty` | 空 messages，验证 error 格式 |

---

## Phase 2：MiniMax 采集

- 覆盖：OpenAI 协议 + Anthropic 协议
- 场景：上表全部场景
- 状态：待 Phase 1 完成

---

## Phase 3：GLM 采集

- 覆盖：OpenAI 协议 + Anthropic 协议
- 场景：上表全部场景
- 状态：待 Phase 2 完成

---

## Phase 4：DeepSeek 采集

- 覆盖：OpenAI 协议 + Anthropic 协议
- 场景：上表全部场景
- 状态：待 Phase 3 完成

---

## Phase 5：结果分析

- 分析各 provider 的 reasoning 参数支持情况
- 分析 Cache 字段的格式差异
- 分析 Thinking 内容格式差异
- 状态：待 Phase 4 完成

---

## Phase 6：README 指引

- 更新 `~/code/closeclaw-test/tests/fixtures/llm/v2/README.md`
- 包含目录结构说明、顶层字段说明、provider 差异表格
- 状态：待开始

## 脚本用法（capture_fixtures.py）

位置：`~/code/closeclaw-test/tests/fixtures/llm/v2/capture_fixtures.py`

### 两种运行模式

**单条采集**（指定 model + scenario）：
```bash
cd ~/code/closeclaw-test/tests/fixtures/llm/v2
python3 capture_fixtures.py --provider glm --model glm-5.1 --scenario simple --api-key YOUR_KEY
```

**批量采集**（遍历 provider 下所有模型 × 所有场景）：
```bash
cd ~/code/closeclaw-test/tests/fixtures/llm/v2
python3 capture_fixtures.py --provider glm --all --api-key YOUR_KEY
```

> 注意：批量模式默认跳过已存在且文件大小 > 100 字节的 fixture（避免重复采集覆盖有效数据）。如需强制重采，先删除目标文件。

### 参数说明

| 参数 | 必填 | 说明 |
|------|------|------|
| `--provider` | ✅ | provider 名，支持：`minimax` / `glm` / `deepseek` |
| `--api-key` | ✅ | API key，或通过环境变量 `LLM_API_KEY` |
| `--model` | （单条模式必填） | 模型名，如 `glm-5.1` |
| `--scenario` | （单条模式必填） | 场景名，详见下方场景列表 |
| `--all` | （批量模式） | 遍历该 provider 下全部模型 × 全部场景 |
| `--output-base` | ❌ | 输出根目录，默认 `/home/admin/code/closeclaw-test/tests/fixtures/llm/v2` |

### 场景列表（`--scenario` 可用值）

**OpenAI 协议**（31个场景）：

| scenario | 说明 | provider 限制 |
|----------|------|-------------|
| `simple` | 基础 single-turn 对话 | — |
| `streaming` | SSE stream，验证 chunk 格式 | — |
| `multi-turn` | 多轮对话 | — |
| `cache` | prompt_tokens_details.cached_tokens | — |
| `minimax-reasoning-split` | MiniMax reasoning_split=true | minimax |
| `glm-thinking` | GLM thinking enabled | glm |
| `glm-thinking-disabled` | GLM thinking disabled | glm |
| `glm-tool-use-streaming` | GLM 流式工具调用（tool_stream=True） | glm（GLM-5/4.7/4.6，**GLM-5.1 不支持**） |
| `glm-tool-result` | GLM 工具调用多轮（Round 1 → tool_result → Round 2） | glm |
| `deepseek-thinking-high` | DeepSeek reasoning_effort=high | deepseek |
| `deepseek-thinking-disabled` | DeepSeek reasoning_effort=low | deepseek |
| `tool-use` | 触发工具调用（finish_reason=tool_calls） | — |
| `tool-result` | 工具调用多轮（Round 1 → tool_result → Round 2） | — |
| `tool-use-streaming` | Streaming 下工具调用 | — |
| `error-auth` | 无效 API key | — |
| `error-model` | 无效 model name | — |
| `error-empty` | 空 messages | — |
| `error-tool-format` | tools 格式错误（缺少 function.name） | — |

**Anthropic 协议**（12个场景）：

| scenario | 说明 | provider 限制 |
|----------|------|-------------|
| `anthropic-simple` | 基础请求，验证 content[].type=text | — |
| `anthropic-thinking` | Anthropic thinking block（MiniMax 默认出现） | minimax |
| `glm-anthropic-tool-use` | GLM Anthropic 工具调用（content[].type=tool_use） | glm |
| `glm-anthropic-tool-result` | GLM Anthropic 工具调用多轮 | glm |
| `anthropic-streaming` | SSE stream，验证事件序列 | — |
| `anthropic-tool-use` | Anthropic 工具调用 | minimax |
| `anthropic-tool-result` | Anthropic 工具调用多轮 | minimax |
| `anthropic-cache` | cache_control:ephemeral 主动缓存 | minimax |
| `anthropic-error-auth` | Anthropic 端点无效 key | — |
| `anthropic-error-model` | Anthropic 端点无效 model | — |
| `anthropic-error-empty` | Anthropic 端点空 messages | — |
| `anthropic-tool-use-streaming` | Anthropic SSE 流式工具调用 | minimax |

### 输出结构

```
tests/fixtures/llm/v2/{provider}/
├── openai/
│   ├── {model}-simple.json          # 普通 JSON（单次响应）
│   ├── {model}-streaming.txt        # 原始 SSE 文本
│   ├── {model}-{scenario}-meta.json # 流式场景的 metadata
│   └── {model}-{scenario}.json
└── anthropic/
    ├── {model}-{scenario}.json
    └── {model}-{scenario}.txt       # 流式原始 SSE
```

### 环境要求

- Python 3.8+
- 无第三方依赖（仅使用标准库）
- 工作目录：`~/code/closeclaw-test/tests/fixtures/llm/v2/`

---

## 当前阻塞

- 设计文档 `design/36-llm-session-enhancements.md` 中 R1/R2/R3 待 owner 确认
- Phase 0 文档深挖待 owner 提供 DeepSeek 文档链接

---

## Phase 6：README 指引

- 更新 `~/code/closeclaw-test/tests/fixtures/llm/v2/README.md`
- 包含目录结构说明、顶层字段说明、provider 差异表格
- 状态：待开始