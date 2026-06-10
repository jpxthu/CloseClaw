# LLM 模块

## 概述

LLM 模块为 CloseClaw 提供统一的多供应商、多协议、多模型 LLM 调用能力，将不同 AI 供应商的 API 差异封装在多层抽象之后，对上层暴露一致的聊天交互接口，支持文本对话、推理过程、工具调用和流式输出。

## 架构

LLM 模块采用五层分离架构，每层只做一件事，层间通过标准类型传递。模块支持两种协议：OpenAI 协议（内容以纯文本字符串承载，推理过程嵌入 XML 标签）和 Anthropic 协议（内容以类型化结构数组承载，推理过程和工具调用为独立内容块）。Protocol 层负责屏蔽两种协议的序列化差异。

**前缀稳定性原则**：同一 session 内连续多次 API 调用之间，请求的前缀部分（静态 system prompt + 历史消息）必须完全相同，只有尾部新增内容变化——这是前缀缓存生效的前提。任何在请求前缀中注入可变内容的做法都会系统性破坏 KV cache，导致所有 token 按全价计费。此原则贯穿 system prompt 静态/动态分离、消息历史尾部缓存标记、以及 cache adapter 的缓存控制参数注入。

> **模块边界**：LLM 模块以 LLM Client 为对外边界。图上方的 Session 层属于上层应用模块，不在 LLM 模块范围内，此处画出是为了展示 LLM Client 的调用上下文。消息渲染（Rendering Layer / processor_chain）完全在 LLM 模块之外，通过 Session 层间接消费 LLM 的响应内容。

```
外部调用方
  ├─ Session 层（构建请求、消费响应）
  └─ config_wizard（模型发现）
  │
  ▼  ← LLM 模块边界
LLM Client（UnifiedChatClient）—— 统一入口
  │
  ├─ 缓存适配器（请求前置处理，标记静态区缓存控制参数）
  │
  ├─ Plugin Pipeline（模型专属行为注入，1:1 绑定）
  │    before_request → after_response → on_stream_event
  │
  ├─ ModelInterpreter（字段归一化）
  │    将协议原生字段（如 reasoning_content）归一化为统一内容块
  │
  ├─ Protocol（OpenAI / Anthropic）
  │    请求序列化、响应解析、流式 SSE 解析
  │
  └─ Provider（HTTP 基础设施）
       base_url、api_key、HTTP 发送、厂商错误映射
```

### 分层职责

| 层 | 职责 | 业务逻辑 |
|----|------|---------|
| Provider | 保存访问凭据和端点地址，发送 HTTP 请求，映射厂商错误码 | 否 |
| Protocol | 按协议标准序列化请求、解析响应、解析流式 SSE 事件 | 是（协议标准逻辑） |
| ModelInterpreter | 将协议原生字段归一化为统一内容块（如 reasoning_content → Thinking block） | 是（模型特有逻辑） |
| Plugin | 模型专属行为注入（注入额外参数、过滤内容块、处理流式事件） | 是（业务行为） |

### 统一内容块

所有协议的响应内容归一化为四种内容块类型：文本块、推理块、工具调用块、工具结果块。无论上游是 OpenAI 协议还是 Anthropic 协议，上层业务只看到这四种类型。

### 流式事件

流式输出通过统一事件模型传递：内容块开始、内容增量、内容块结束、消息结束、错误事件。屏蔽 OpenAI SSE 和 Anthropic SSE 在事件粒度上的差异。

### 模型发现

LLM 模块通过独立的模型发现（ModelDiscovery）服务提供模型发现能力：消费各 Provider 的 ModelLister 实现，通过 `/models` 端点动态探测可用模型，结合本地缓存和内嵌知识库，自动填充模型的推荐参数（上下文窗口、最大输出、推理标记等）。此能力独立于对话调用链路，主要用于配置阶段的模型选择。详见 [model-discovery](model-discovery.md)。

**子功能文档**：
- [cache-adapter](cache-adapter.md) — 跨供应商统一缓存策略，最大化静态区 system prompt 的缓存折扣
- [model-discovery](model-discovery.md) — 从供应商 API 动态发现可用模型，结合知识库和缓存自动填充推荐参数
- [provider-config-wizard](provider-config-wizard.md) — CLI 交互式向导：选 provider、输入凭据、发现模型、写配置

## 数据流

```
外部调用方（Session 层）构建请求
  → LLM Client 接收统一请求
    → 缓存适配器 apply（标记静态区缓存控制参数）
    → Plugin Pipeline 调用 before_request（注入模型特定参数）
    → ModelInterpreter 注入请求参数（模型特有字段）
      → Protocol 层序列化请求体
        → Provider 层发送 HTTP 请求
          ← 供应商返回原始响应
        ← Provider 返回 JSON
      ← Protocol 层解析为内部响应结构
    ← ModelInterpreter 将内部响应归一化为统一响应（内容块 + 用量 + 结束原因）
  ← Plugin Pipeline 调用 after_response（后处理内容块）
← LLM Client 返回统一响应给外部调用方

外部调用方将响应内容写入消息数组后，上层 Rendering Layer / processor_chain 负责读取并渲染（不在 LLM 模块范围内）。
```

流式调用路径：Provider 以 SSE 流读取原始数据块 → Protocol 层通过状态机将原始事件转换为统一流式事件 → ModelInterpreter 对流事件做额外归一化 → Plugin 对流事件做过滤/加工 → 上层逐事件消费。

## 模块关系

LLM 模块以 LLM Client（UnifiedChatClient）为统一入口，对外暴露对话调用和模型发现两类能力。

- **上游**：Session 层（通过 LLM Client 发起对话请求、消费统一响应）、system_prompt 模块（为缓存适配器提供静态区和动态区内容）、CLI 配置命令（触发 Provider 配置向导）
- **下游**：各供应商 API（HTTP 调用）、文件系统（凭据文件读写、模型列表缓存）
- **无关**：processor_chain（消息处理链，在 Rendering Layer 内部；LLM 模块不直接与之交互，而是通过 Session 层间接传递响应内容）、agent/subagent 调度层（通过 Session 间接调用 LLM，不直接依赖 Provider/Protocol 细节）
