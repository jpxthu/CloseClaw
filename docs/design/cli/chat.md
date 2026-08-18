# CLI Chat

## 概述

CLI Chat 是 terminal 消息渠道的对话交互功能，通过 `closeclaw chat` 命令启动。它实现 IMPlugin trait，以 platform="terminal" 注册到 Gateway 的 Plugin Registry，将终端输入输出接入完整的出入站消息链路。此功能依赖 daemon 已运行。

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
  └─ 普通文本 → Session → LLM → ContentBlock[]

出站
  ContentBlock[]
    ↓
  Processor Chain 出站（VerbosityFilter → DslParser → OutboundRawLog）
    ↓
  TerminalPlugin
    ↓
  TerminalRenderer 渲染
    ↓
  RenderedOutput（ANSI 文本数据）
    ↓
  TerminalPlugin 发送到 stdout
```

> **流式路径**：LLM 流式输出时，ContentBlock[] 走与批量相同的预处理路径（Verbosity → DslParser 零开销透传）。流式渲染由 IM Adapter 模块的流式渲染组件驱动，TerminalPlugin 逐行产生增量 RenderedOutput 后立即写入 stdout，不经过 TerminalRenderer 批量渲染。流式结束后，DslParser 完整解析 → OutboundRawLog 写入出站日志。详见 [IM Adapter 流式渲染](../im_adapter/streaming-render.md)。

### 入站：TerminalAdapter

TerminalAdapter 从 stdin 读取用户输入，封装为 NormalizedMessage（字段定义见 [common 共享类型](../common/shared-types.md)）。terminal 渠道的字段取值：

terminal 渠道 NormalizedMessage 取值：

- platform = "terminal"
- sender_id = 当前用户系统 UID
- peer_id = "cli"
- account_id = "owner"
- content = 原始输入文本
- message_type = text

其余字段（thread_id、media_refs、timestamp）按默认值：thread_id 为空，media_refs 为空列表，timestamp 取系统时间。

消息过滤规则与其他渠道一致：空内容不产出 NormalizedMessage。

### 出站：TerminalRenderer

TerminalRenderer 接收 ContentBlock[]（定义见 [common ContentBlock](../common/shared-types.md#contentblock)）和 DSL 解析结果，转换为 ANSI 格式的 RenderedOutput。TerminalPlugin 通过 send 方法将 RenderedOutput 写入 stdout。渲染与发送分离，遵循 IM Adapter 框架的设计原则。详细渲染策略见 [Terminal Renderer](renderer.md)。

### Session 与 Agent 指定

用户通过 `--agent-id` 指定目标 agent。agent 级别的 session 隔离由 per-agent SessionManager 提供——不同 agent 各自维护独立的 key_registry 和 session 集合，同一用户对不同 agent 的对话天然隔离，无需 agent_id 参与 session_key 计算。

通过 `/stop` 斜杠指令强制终止当前运行（固定 Forceful，级联终止子 session；停运行不停会话，详见 [slash/session-management.md](../slash/session-management.md)）；不活跃的 session 由 SessionManager 自动归档。

## 数据流

```
stdin 逐行/逐段读取用户输入
  ↓
TerminalAdapter 解析并封装 NormalizedMessage（内部含空内容过滤，空内容不产出 NormalizedMessage）。终端字段取值见上文架构节。
  ↓
Processor Chain 入站
  ├── RawLog：记录原始用户输入文本到日志
  ├── SessionRouter：根据平台、用户和端点计算 session key
  └── ContentNormalizer：文本标准化（去除控制字符和 ANSI 转义序列、压缩空行、去尾空格）
  ↓
Processor Chain 产出的处理后消息 → Gateway 路由
  ├── / 开头 → SlashDispatcher（与飞书等渠道共享同一套）
  └── 普通文本 → Session → LLM → ContentBlock[]
  ↓
Processor Chain 出站
  ├── VerbosityFilter：按 Session Verbosity 等级过滤 ContentBlock[]
  ├── DslParser：扫描并剥离 DSL 指令行到 metadata（与通用 DslParser 行为一致，不按平台过滤）
  └── OutboundRawLog：写入出站日志
  ↓
TerminalPlugin 调用 TerminalRenderer 执行渲染
  ↓
终端检测 + DSL 预处理
  ├── 终端能力检测：确定渲染模式（ANSI / 纯文本）+ 获取终端可用宽度
  └── DSL 交互元素预处理：按钮 / 选择器 → 纯文本提示行；其他 DSL → 忽略
  ↓
逐块渲染
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

> **流式路径**：LLM 流式输出时，不走 TerminalRenderer 批量渲染路径。ContentBlock[] 经统一预处理（Verbosity → DslParser 零开销透传）后，IM Adapter 流式渲染组件驱动，TerminalPlugin 逐行产生增量 RenderedOutput 后立即写入 stdout。流式结束后，DslParser 完整解析 → OutboundRawLog 写入出站日志。详见 [IM Adapter 流式渲染](../im_adapter/streaming-render.md)。

## 模块关系

- **上游**：操作系统 stdin（用户输入）、Gateway（通过 IMPlugin trait 调用 TerminalPlugin 出站）
- **下游**：Gateway（接收 NormalizedMessage 入站路由）、stdout（TerminalPlugin.send() 输出渲染结果）
- **与模块内其他子功能**：使用 TerminalRenderer 完成出站渲染，renderer 文档定义详细的块类型渲染规则
- **无关**：CLI Admin（Admin 命令不走消息链路，不经过 TerminalPlugin）、IM Adapter 的具体平台实现（terminal 渠道与其平级）
