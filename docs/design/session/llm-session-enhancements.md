# LLM 会话增强

## 概述

LLM 会话增强是 session 模块中处理每次 LLM API 调用的增强层，覆盖四个维度：流式输出推送、推理强度控制、缓存命中统计、以及 Thinking 内容管理。这些增强贯穿每次会话交互的 API 调用周期，确保会话在与不同 provider 交互时行为一致。

## 架构

LLM 会话增强在 ConversationSession 与 LLM Provider 之间的调用链路上插入处理逻辑：

```
ConversationSession
  │
  ├── 构建请求 ──────────────────────────────────────────
  │     ├── 消息历史 + system prompt
  │     ├── Reasoning Level 注入（config 默认 + 运行时覆盖）
  │     └── stream 标志位
  │
  ├── 调用 LLM ──────────────────────────────────────────
  │     ├── 流式路径：遍历 provider 链选择可用流式 provider
  │     │     ├── 逐 chunk 推送到流式推送抽象层
  │     │     └── Done chunk 到达 → 提取用量信息 → 累加统计
  │     │
  │     └── 非流式路径：直接调用 provider 获取完整响应
  │           └── 返回完整响应
  │
  └── 响应后处理 ────────────────────────────────────────
        ├── Thinking 内容作为独立 block 保留
        ├── 写入 message history（含 Thinking block）
        └── 累加用量统计
```

### 流式输出

会话支持流式和非流式两条路径，通过请求中的 `stream` 标志位选择。

**流式路径**通过 `StreamingSink` trait 实现平台无关的实时推送。Session 层持有 `StreamingSink` 实例，将 Text chunk 逐条推送，不感知下游 IM 类型。流结束时发送 Done 通知（携带模型名和用量信息），流错误时发送 Error 通知。

各 LLM provider 通过各自的流式接口实现 SSE 事件解析，处理各自的事件格式差异。FallbackClient 统一调度：遍历 provider chain，对支持流式的 provider 调用其流式接口，无可用流式 provider 时降级为非流式调用后逐字推送。

**StreamingSink trait** 定义三个核心方法：发送文本片段、流结束通知（携带 usage）、流错误通知。各 IM 适配器实现此 trait 即可接入流式推送。

### Reasoning Level 推理控制

Reasoning Level 控制 LLM 的推理深度，通过 config 默认值 + 运行时指令覆盖两级入口生效。

**四个等级**：Low、Medium、High、Max。High 为各 provider 默认值。不支持的等级自动降级（如 Max 在不支持的模型上降为 High），降级时记录日志但不对用户主动通知。

**两级入口**：
- **Config 配置**：`llm.reasoning_level` 设置全局默认值
- **运行时指令**：`/reasoning` 无参数时查询当前等级，`/reasoning [level|off]` 修改当前 session 等级，覆盖 config 默认值，不回写配置文件

**Provider 注入**：各 provider builder 持有自己的参数映射表，将 ReasoningLevel 转换为 provider 原生的 reasoning 参数。不同 provider 支持的参数格式不同——有的用 `reasoning_effort` 字段，有的用 `thinking.type` 开关，部分 provider 不支持 reasoning 控制。

### Cache Hit 缓存统计

会话维护跨轮次的 `RunningStats`，每次 API 调用完成后累加用量数据。

**Usage 扩展**：除基础的 prompt/completion/total tokens 外，增加 `cache_read_tokens`（命中缓存的输入 token）和 `cache_write_tokens`（新写入缓存的 token）。

**RunningStats** 跨轮次累加所有用量，支持查询缓存命中率（cache_read / total_input）。流式过程中 RunningStats 在每次 Done chunk 到达时更新（此时才有完整 usage），中途查询返回上一次累加值。

不同 provider 的缓存字段路径不同——有的在 `usage.prompt_tokens_details.cached_tokens`，有的在 `usage.cache_read_input_tokens`。各 provider builder 负责从各自响应格式中提取并统一填入 Usage 结构。

### Thinking 内容管理

LLM 响应中的 Thinking 内容以独立 block 形式保留在消息历史中，作为上下文的一部分参与后续对话。流式过程中 Thinking 内容不通过 StreamingSink 发送给用户。

**消息历史策略**：Thinking block 保留在 message history 中，参与 token 计数和上下文窗口管理。理由：Thinking 内容蕴含模型的推理链，后续对话中可供模型参考，提升推理连续性。

**两道清理防线**（在消息发送给 API 前执行，处理流式合并过程中产生的边界异常）：
- **孤立 Thinking 清理**：流式传输中每个 content block 产生独立消息。若合并后出现仅有 Thinking block、无同消息 ID 的 non-Thinking 兄弟消息可合并的情况，移除该孤立消息。
- **末尾 Thinking 清理**：API 不允许 assistant 消息以 Thinking block 结尾。若最后一条 assistant 消息的末尾 block 为 Thinking，从末尾移除直到遇到 non-Thinking block。若全部为 Thinking，替换为占位空文本。

**可见性策略**：Thinking 内容属于内部推理，在消息传输和存储层面保留，但在终端展示层面可控制显示——主终端仅展示推理状态指示（如 shimmer），完整内容在详情面板中按需查看。

## 数据流

### 一次完整的增强调用

```
请求进入
  │
  ├── Reasoning Level 解析
  │     └── session 运行时覆盖 > config 默认 > None
  │
  ├── Provider 参数注入
  │     ├── Reasoning Level → provider 原生 reasoning 参数
  │     └── 不支持 → 降级或跳过
  │
  ├── 路径选择
  │     ├── stream=true → 流式路径
  │     │     ├── provider 流式调用
  │     │     ├── 每 chunk → 推送文本片段
  │     │     ├── Done chunk → 流结束通知（携带用量）
  │     │     │                → 累加用量统计
  │     │     └── Error chunk → 错误通知
  │     │                       → 已发送 chunks 保留
  │     │                       → message history 不写入
  │     │
  │     └── stream=false → 非流式路径
  │           └── provider 非流式调用
  │
  └── 响应后处理
        ├── Thinking 内容作为独立 block 保留
        ├── 写入 message history（含 Thinking block）
        └── 累加用量统计
```

### Reasoning Level 生效链路

```
config.yaml: llm.reasoning_level: high
                │
                ▼
        SessionManager 读入默认值
                │
    ┌───────────┴───────────┐
    │                       │
    ▼                       ▼
无运行时覆盖               /reasoning medium
    │                       │
    ▼                       ▼
使用 config 默认        session 临时覆盖
    │                       │
    └───────────┬───────────┘
                ▼
        Provider builder 映射
          High → reasoning_effort: "high"（DeepSeek）
          High → thinking.type: "enabled"（GLM）
          High → N/A（MiniMax，不支持）
                │
                ▼
        注入 LLM API 请求体
```

### Cache Hit 统计链路

```
API 响应返回
  │
  ├── Provider 提取缓存字段（各 provider 路径不同）
  │     ├── DeepSeek：命中缓存的输入 token
  │     ├── GLM：prompt_tokens_details 中的 cached_tokens
  │     └── MiniMax：cache_read_input_tokens（Anthropic 协议）
  │                  或 prompt_tokens_details.cached_tokens（OpenAI 协议）
  │
  ▼
Usage 结构（含缓存命中数、缓存写入数）
  │
  ▼
RunningStats 累加
  ├── 缓存命中输入累加
  ├── 缓存写入累加
  └── 总输入 token 累加
```

## 模块关系

### 上游

- **ConversationSession**：调用增强层构建 LLM 请求、处理响应，提供 Reasoning Level 运行时覆盖和 RunningStats 存储。
- **SessionManager**：创建 session 时注入 config 中的默认 reasoning level。
- **Slash Command**：`/reasoning` 指令运行时修改 session 的 reasoning level。

### 下游

- **LLM Provider**：接收增强后的请求，返回原始响应。各 provider builder 负责 reasoning 参数注入和 cache 字段提取。
- **StreamingSink**（trait）：流式模式下向 IM 适配器推送实时输出。各 IM 适配器实现此 trait 接入。
- **RunningStats**：接收每次调用的 Usage 数据，累加统计。

### 无关

- **Permission 模块**（无调用关系）：权限检查在 Gateway 层，在 LLM 调用之前完成。
- **Compaction 模块**（无调用关系）：压缩处理消息历史，与 LLM 调用的增强逻辑不交叉。
- **System Prompt Builder**（无调用关系）：System prompt 组装在 session 创建/恢复时完成，不经过增强层。
