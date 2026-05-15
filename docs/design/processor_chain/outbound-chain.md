# 出站链路

## 概述

出站链路负责将 LLM 输出从统一格式转换为各 IM 平台可发送的消息。链路分两个阶段：Processor 链的 DSL 解析阶段，和 Rendering Layer 的平台渲染阶段。

## 架构

**阶段一：Processor 链（出站方向）**

出站 Processor 链只保留 DslParser：

```
DslParser（priority: 10）
  → 遍历消息内容，识别 DSL 指令行
  → 解析结果写入 ProcessedMessage.metadata
```

**阶段二：Rendering Layer**

```
Renderer(messages[], dsl_result)
  → 按 platform 选择对应 Renderer 实现
  → 输出平台原生格式
```

Rendering Layer 不经过 Processor 链，直接从 Session 读取消息数组并完成渲染。DslParser 的解析结果由 Gateway 从 metadata 中提取后传给 Renderer。

## 数据流

```
UnifiedResponse（LLM 输出）
  ↓ 写入 Session
Session 消息数组
  ↓ Gateway 调度 Processor 链
DslParser 解析 DSL → metadata["dsl_result"]
  ↓ Gateway 提取 DSL 结果
Renderer.render(messages, dsl_result)
  ↓ 平台格式输出
RenderedOutput { msg_type, payload }
  ↓
IM Adapter.send(chat_id, payload)
  ↓
IM 用户
```

关键判断点：
- DslParser 遍历消息内容时，遇到 DSL 行则解析为结构化指令，否则跳过
- Renderer 根据目标 platform 选择对应的平台实现
- 渲染输出包含 msg_type 和平台 JSON payload

## 模块关系

- **上游**：Session（提供消息数据）、LLM Provider（生成 UnifiedResponse）
- **下游**：IM Adapter（接收 RenderedOutput 并发送）
- **链内**：DslParser 是出站链唯一的 Processor
