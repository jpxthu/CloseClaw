# CLI Chat

## 概述

CLI Chat 是 terminal 消息渠道的对话交互功能。它实现 IMPlugin trait，以 platform="terminal" 注册到 Gateway 的 Plugin Registry，将终端输入输出接入完整的出入站消息链路。

## 架构

CLI Chat 的实现实体是 TerminalPlugin（IMPlugin trait 的 terminal 渠道实现），包含 TerminalAdapter（入站解析）和 TerminalRenderer（出站渲染）两个组件。

```
入站
  stdin
  → TerminalAdapter
  → NormalizedMessage
  → Processor Chain 入站（RawLog → SessionRouter → ContentNormalizer）
  → Gateway 路由
  ├─ / 开头 → SlashDispatcher
  └─ 普通文本 → Session
  → LLM

出站
  ContentBlock[]
  → Processor Chain 出站（DslParser）
  ↓
  TerminalPlugin
    ↓
    TerminalRenderer 渲染 → RenderedOutput（ANSI 文本数据）
    ↓
    发送 → stdout
```

> **流式路径**：LLM 流式输出时，不走上述经 Processor Chain 的批量渲染路径。Gateway 通过 IMPlugin trait 的流式默认方法驱动 DefaultStreamingRenderer，直接逐行产生增量 RenderedOutput 后由 TerminalPlugin 发送到 stdout。流式文本输出不经 Processor Chain——文本增量无需 DslParser 参与。流式结束后 Gateway 记录出站日志。详见 [IM Adapter 流式渲染](../im_adapter/streaming-render.md)。

### 入站：TerminalAdapter

TerminalAdapter 从 stdin 读取用户输入，封装为 NormalizedMessage：

- platform 固定为 "terminal"
- sender_id 取当前用户的系统 UID，用于 session 路由
- peer_id 固定为 "cli"，意味着每个用户一个 session
- thread_id 不适用（终端无话题概念），固定为空
- account_id 由 sender_id 映射得到（本地用户默认为 Owner）
- content 为原始输入文本（支持多行，以空行分隔消息边界）
- message_type 固定为 text
- media_refs 不适用（终端仅支持文本输入），固定为空
- quoted_message 不适用，固定为空
- timestamp 取消息接收时的系统时间

消息过滤规则与其他渠道一致：空内容不产出 NormalizedMessage。

### 出站：TerminalRenderer

TerminalRenderer 接收 ContentBlock[] 和 DSL 解析结果，转换为 ANSI 格式的 RenderedOutput。TerminalPlugin 通过 send 方法将 RenderedOutput 写入 stdout。渲染与发送分离，遵循 IM Adapter 框架的设计原则。详细渲染策略见 [Terminal Renderer](renderer.md)。

### Session 与 Agent 指定

用户通过 `--agent-id` 指定目标 agent。agent 级别的 session 隔离由 per-agent SessionManager 提供——不同 agent 各自维护独立的 key_registry 和 session 集合，同一用户对不同 agent 的对话天然隔离，无需 agent_id 参与 session_key 计算。

通过 `/stop` 斜杠指令终止会话；不活跃的 session 由 SessionManager 自动归档。

## 数据流

```
stdin 逐行/逐段读取用户输入
  ↓
TerminalAdapter 解析并封装 NormalizedMessage（内部含空内容过滤，空内容不产出 NormalizedMessage）
  │  { platform: "terminal", sender_id, peer_id: "cli", account_id, thread_id, content, message_type, media_refs, quoted_message, timestamp }
  ↓
Processor Chain 入站
  ├── RawLog：记录原始输入到日志
  ├── SessionRouter：根据平台、用户和端点计算 session key
  └── ContentNormalizer：文本标准化（去除控制字符和 ANSI 转义序列、压缩空行、去尾空格）
  ↓
Processor Chain 出产的处理后消息 → Gateway 路由
  ├── / 开头 → SlashDispatcher（与飞书等渠道共享同一套）
  └── 普通文本 → Session → LLM → ContentBlock[]
  ↓
Processor Chain 出站
  └── DslParser：扫描并剥离 DSL 指令行到 metadata（与通用 DslParser 行为一致，不按平台过滤）
  ↓
Gateway 记录出站日志
  ↓
TerminalPlugin 调用 TerminalRenderer 执行渲染
  ├── Text 块 → ANSI 格式化文本
  ├── Thinking 块 → 折叠块（[Thinking] … [end of thinking]）
  ├── ToolUse 块 → 工具调用展示
  ├── ToolResult 块 → 工具结果展示
  ├── Image 块 → 占位符 "[image: name]"
  ├── Audio 块 → 占位符 "[audio: name]"
  └── File 块 → 占位符 "[file: name]"
  ↓
RenderedOutput { msg_type: "text", payload: ANSI 文本 }
  ↓
TerminalPlugin 的 send 方法写入 stdout
```

> **流式路径**：LLM 流式输出时，不走上述 TerminalRenderer 批量渲染路径。Gateway 通过 IMPlugin trait 的流式默认方法驱动 DefaultStreamingRenderer，直接逐行产生增量 RenderedOutput 后立即由 TerminalPlugin 发送到 stdout。流式文本不经 Processor Chain——文本增量直接由 Gateway 驱动流式引擎逐行产生，无需 DslParser 参与。流式结束后 Gateway 记录出站日志。详见 [IM Adapter 流式渲染](../im_adapter/streaming-render.md)。

## 模块关系

- **上游**：操作系统 stdin（用户输入）、Gateway（通过 IMPlugin trait 调用 TerminalPlugin 出站）
- **下游**：Gateway（接收 NormalizedMessage 入站路由）、stdout（TerminalPlugin.send() 输出渲染结果）
- **与模块内其他子功能**：使用 TerminalRenderer 完成出站渲染，renderer 文档定义详细的块类型渲染规则
- **无关**：CLI Admin（Admin 命令不走消息链路，不经过 TerminalPlugin）、IM Adapter 的具体平台实现（terminal 渠道与其平级）
