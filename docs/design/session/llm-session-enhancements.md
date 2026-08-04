# LLM 会话增强

## 概述

LLM 会话增强是 session 模块中处理每次 LLM API 调用的增强层，覆盖四个维度：流式输出推送、推理强度控制、用量统计、以及 Thinking 内容管理。这些增强贯穿每次会话交互的 API 调用周期，确保会话在与不同 provider 交互时行为一致。

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
  │     │     ├── 逐 chunk 组装 ContentBlock[]
  │     │     └── Done chunk 到达 → 提取用量信息（暂存）
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

**流式路径**：Session 层接收 LLM 流式 chunk，逐块组装 ContentBlock[] 并通过 Gateway 统一出站路径（Verbosity → Processor Chain → 出站日志）实时推送至 IM Adapter 渲染发送。Session 层持有 ContentBlock[] 组装状态，不感知下游 IM 类型和渲染模式。

流式输出过程中发生错误时，已输出的文本片段保留给用户，同时展示错误提示。不完整的响应不写入 message history。

各 LLM provider 通过各自的流式接口实现 SSE 事件解析，处理各自的事件格式差异。

### Reasoning Level 推理控制

Reasoning Level 控制 LLM 的推理深度，通过 config 默认值 + 运行时指令覆盖两级入口生效。

**四个等级**：Low、Medium、High、Max。High 为各 provider 默认值。不支持的等级自动降级（如 Max 在不支持的模型上降为 High），降级时记录日志但不对用户主动通知。

**两级入口**：
- **Config 配置**：`llm.reasoning_level` 设置全局默认值
- **运行时指令**：`/reasoning` 无参数时查询当前实际生效等级（含 provider 降级后的值），`/reasoning [level|off]` 修改当前 session 等级，覆盖 config 默认值，不回写配置文件。输入非法档位值时忽略输入并回显当前生效的档位

**Provider 注入**：各 provider builder 持有自己的参数映射表，将 ReasoningLevel 转换为 provider 原生的 reasoning 参数。不同 provider 支持的参数格式不同——有的用 `reasoning_effort` 字段，有的用 `thinking.type` 开关，部分 provider 不支持 reasoning 控制。

部分供应商的模型设计上总是输出推理内容（如 DeepSeek 的 thinking 无法真正关闭，MiMo 在所有场景下均输出推理），`/reasoning off` 在这些供应商上仅将推理强度降至最低档位。

### 用量统计

会话维护跨轮次的 `RunningStats`，每次 API 调用完成后累加用量数据。

**Usage 扩展**：除基础的 prompt/completion/total tokens 外，增加 `cache_read_tokens`（命中缓存的输入 token）、`cache_write_tokens`（新写入缓存的 token）和 `reasoning_tokens`（推理消耗的 token，与文本输出分开统计）。若 API 响应不携带缓存字段则对应字段显示为 0。

**RunningStats** 跨轮次累加所有用量，保留上一轮快照用于命中率对比。支持查询缓存命中率（cache_read / total_input）。流式过程中 RunningStats 在每次 Done chunk 到达时更新（此时才有完整 usage），中途查询返回上一次累加值。会话结束时 RunningStats 清零。

**缓存命中率下降检测**：

增强层在每次 API 调用后比对本轮与上一轮的缓存命中率（`cache_read_tokens` / `total_input_tokens`）。命中率基于 RunningStats 中保留的上一轮快照与本轮增量计算，若降幅超过可配置阈值则标记为缓存命中率下降事件。用户可通过用量查询（如 `/status` 指令）查看该事件及可能的下降原因（如上下文变更、缓存 TTL 过期等）。

检测仅基于 API 响应中已有的缓存统计字段，不做额外的请求指纹计算或消息注入。

### Thinking 内容管理

LLM 响应中的 Thinking 内容以独立 block 形式保留在消息历史中，作为上下文的一部分参与后续对话。流式过程中 Thinking 内容不随 Text block 实时输出给用户，在流式结束后作为完整 Thinking block 保留在 ContentBlock[] 中。

**消息历史策略**：Thinking block 保留在 message history 中，参与 token 计数和上下文窗口管理。理由：Thinking 内容蕴含模型的推理链，后续对话中可供模型参考，提升推理连续性。

**两道清理防线**（仅在构造发送给 LLM API 的消息列表时执行，不改变存储的 message history。先执行孤立清理再执行末尾清理）：
- **孤立 Thinking 清理**：流式合并过程中，同一消息 ID 下的 Thinking block 可能因 provider 行为差异而与其他 block 分属不同消息。清理时移除没有同消息 ID non-Thinking 兄弟 block 的孤立 Thinking 消息。
- **末尾 Thinking 清理**：API 不允许 assistant 消息以 Thinking block 结尾。若发送给 API 的消息列表中最后一条 assistant 消息的末尾 block 为 Thinking，从末尾移除直到遇到 non-Thinking block。若全部为 Thinking，替换为占位空文本。

**可见性策略**：Thinking 内容属于内部推理，在消息传输和存储层面始终保留（供后续对话引用），但在终端展示层面可控制显示。增强层默认不在主终端展示思考过程，通过推理状态指示（如 shimmer）告知用户推理进行中。用户可通过详情面板按需查看完整推理文本。

## 数据流

### 一次完整的增强调用

```
请求进入
  │
  ├── Reasoning Level 解析
  │     └── session 运行时覆盖 > config 默认
  │
  ├── Provider 参数注入
  │     ├── Reasoning Level → provider 原生 reasoning 参数
  │     └── 不支持 → 降级或跳过
  │
  ├── 路径选择
  │     ├── stream=true → 流式路径
  │     │     ├── provider 流式调用
  │     │     ├── 每 chunk → 组装 ContentBlock[] → 实时推送至 IM Adapter
  │     │     ├── Done chunk → ContentBlock[] 完成（携带用量，暂存）
  │     │     └── Error chunk → 错误通知
  │     │                       → 已输出片段保留给用户
  │     │                       → message history 不写入
  │     │
  │     └── stream=false → 非流式路径
  │           └── provider 非流式调用 → 返回完整 ContentBlock[]
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
使用 config 默认        session 运行时覆盖
    │                       │
    └───────────┬───────────┘
                ▼
        Provider builder 映射（各 provider 转换为其原生 reasoning 参数）
                │
                ▼
        注入 LLM API 请求体
```

### Cache Hit 统计链路

```
API 响应返回
  │
  ├── Provider 提取缓存字段（各 provider 路径不同）
  │     └── 各 provider builder 从响应中提取缓存 token 填入统一 Usage 结构
  │
  ▼
Usage 结构（含缓存命中数、缓存写入数、推理消耗数）
  │
  ▼
RunningStats 累加
  ├── 缓存命中输入累加
  ├── 缓存写入累加
  ├── 推理消耗 token 累加
  ├── 总输入 token 累加
  └── 比对本轮与上一轮缓存命中率（cache_read / total_input）→ 降幅超过阈值 → 标记缓存命中率下降事件
```

## 模块关系

### 上游

- **ConversationSession**：调用增强层构建 LLM 请求、处理响应，提供 Reasoning Level 运行时覆盖和 RunningStats 存储。
- **SessionManager**：创建 session 时注入 config 中的默认 reasoning level。
- **Slash Command**：`/reasoning` 指令运行时修改 session 的 reasoning level。

### 下游

- **LLM Provider**：接收增强后的请求，返回原始响应。各 provider builder 负责 reasoning 参数注入和 cache 字段提取。
- **Gateway**（数据流下游）：流式/非流式路径均产出 ContentBlock[] 交付 Gateway，经统一出站管道（Verbosity → Processor Chain → 出站日志 → IM Adapter 渲染）发送。
- **RunningStats**：接收每次调用的 Usage 数据，累加统计。
- **Compaction 模块**（间接数据依赖）：压缩阈值判断读取增强层维护的 RunningStats。

### 无关

- **Permission 模块**（无调用关系）：权限检查不在增强层链路内。
- **System Prompt Builder**（无调用关系）：System prompt 组装在 session 创建/恢复时完成，不经过增强层。
