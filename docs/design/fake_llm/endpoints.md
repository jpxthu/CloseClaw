# 进程与协议端点

## 概述

定义 Fake LLM Server 的进程模型与对外协议端点：独立进程、仅监听本地回环地址、同时提供 OpenAI 与 Anthropic 两种标准对话协议端点及模型发现端点，使编译后的 CloseClaw 二进制以黑盒方式接入。

## 架构

### 进程模型

Fake LLM Server 是独立进程，随测试启动、随测试结束销毁。它不进入发布构建、不随产品分发——仅在测试环境以开发工具形态存在。

- **监听地址**：仅绑定本地回环地址（127.0.0.1），不对公网暴露。端口由测试进程分配（支持自动选取空闲端口后回填配置）
- **生命周期**：测试进程拉起 → 端口就绪 → 被测二进制以该地址为模型端点运行 → 测试结束销毁。每个测试用例可启动独立实例，实例间通过端口隔离
- **无外部依赖**：不依赖真实模型、算力与外部网络，全部响应本地生成

### 接入方式

CloseClaw 侧通过 models.json 的供应商 base_url 指向 Fake LLM Server 地址完成接入（详见 [config](../../config/README.md) 的 models 配置）。Fake LLM Server 对被测二进制表现为一个普通的 LLM 供应商——同样的凭据头、同样的协议请求。凭据不做校验：认证失败等凭据类错误仅由场景显式注入，用于测试错误路径。

### 端点路由

按协议标准路由，两个对话协议共享同一套场景引擎与投递层：

| 端点 | 协议 | 方法 | 职责 |
|------|------|------|------|
| `/v1/chat/completions` | OpenAI | POST | 对话调用（非流式返回完整 JSON，流式返回 SSE chunk 序列） |
| `/v1/messages` | Anthropic | POST | 对话调用（非流式返回 message 对象，流式返回 SSE 事件序列） |
| `/v1/models` | 两者通用 | GET | 模型列表查询（模型发现模拟） |

协议层只做解析与序列化：请求进来后解出协议无关的请求特征交给场景引擎；场景决策出来后按协议形态序列化返回。协议差异（如 OpenAI 的 messages 数组与 Anthropic 的 content 块数组）在这一层归一，场景引擎不感知协议。

### 模型发现模拟

`/v1/models` 返回场景声明的模型列表，用于验证 CloseClaw 的模型发现链路。可注入的形态与对话错误注入共用同一套机制（详见 [delivery](delivery.md)）：

- 正常返回：模型 ID 列表（列表内容可含未知模型，验证 CloseClaw 过滤未知模型的能力）
- 认证失败（HTTP 401）、超时（挂起不响应）、服务端错误（HTTP 5xx）

被测行为——动态探测、过滤未知模型、失败回退内置知识库——的权威定义见 [llm/model-discovery](../llm/model-discovery.md)。

## 数据流

```
测试进程启动 Fake LLM Server（127.0.0.1 + 空闲端口）
  → 端口回填进被测二进制的 models.json 配置
  → 被测二进制发起请求（POST /v1/chat/completions、POST /v1/messages、GET /v1/models 三者之一）
  → 协议层解析请求 → 协议无关的请求特征 → 场景引擎决策
  → 投递层按协议形态序列化响应（JSON / SSE / HTTP 错误）
  ← 被测二进制收到响应
  → 测试结束，进程销毁
```

## 模块关系

- **模块内下游**：[scenario-engine](scenario-engine.md)（解出的请求特征交付给它决策）、[delivery](delivery.md)（投递由它执行）
- **跨模块**：被测二进制的接入点为 [config](../../config/README.md) 的 models.json（base_url 指向本地地址）；模型发现行为锚定 [llm/model-discovery](../llm/model-discovery.md)
- **无关**：发布构建（本模块不进入产品分发）
