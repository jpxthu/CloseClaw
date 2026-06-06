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
  → Session
  → LLM

出站
  ContentBlock[]
  → Processor Chain 出站（DslParser → RawLog）
  → TerminalRenderer
  → stdout
```

### 入站：TerminalAdapter

TerminalAdapter 从 stdin 读取用户输入，封装为 NormalizedMessage：

- platform 固定为 "terminal"
- sender_id 取当前用户的系统 UID，用于 session 路由
- peer_id 固定为 "cli"，意味着每个用户一个 session
- content 为原始输入文本（支持多行，以空行分隔消息边界）

消息过滤规则与其他渠道一致：空内容不产 NormalizedMessage。

### 出站：TerminalRenderer

TerminalRenderer 接收 Processor Chain 出站的 ContentBlock[]，转换为 ANSI 格式文本输出到 stdout。详细渲染策略见 [Terminal Renderer](renderer.md)。

### Session 与 Agent 指定

用户通过 `--agent-id` 指定目标 agent。agent 级别的 session 隔离由 per-agent SessionManager 提供——不同 agent 各自维护独立的 key_registry 和 session 集合，同一用户对不同 agent 的对话天然隔离，无需 agent_id 参与 session_key 计算。

REPL 模式下用户输入 `quit`/`exit` 结束会话；也可以通过 `/stop` 斜杠指令终止。

## 数据流

```
stdin 逐行/逐段读取用户输入
  ↓
空行检测 → 空内容跳过
  ↓
TerminalAdapter 封装 NormalizedMessage { platform: "terminal", sender_id, peer_id: "cli", content, timestamp }
  ↓
Processor Chain 入站
  ├── RawLog：记录原始输入到日志
  ├── SessionRouter：根据平台、用户和端点计算 session key
  └── ContentNormalizer：清洗 stdin 残留控制字符 + 标准化格式
  ↓
ProcessedMessage → Gateway 路由
  ├── / 开头 → SlashDispatcher（与飞书等渠道共享同一套）
  └── 普通文本 → Session → LLM → ContentBlock[]
  ↓
Processor Chain 出站
  ├── DslParser：识别交互 DSL（按钮等终端不支持的类型记录警告、跳过）
  └── RawLog：记录出站到日志
  ↓
TerminalRenderer 接收内容块和 DSL 结果进行渲染
  ├── Text 块 → ANSI 格式化文本
  ├── Thinking 块 → 折叠块（[Thinking] … [end]）
  ├── ToolUse 块 → 工具调用展示
  ├── ToolResult 块 → 工具结果展示
  └── 不支持的内容（图片、语音等）→ 占位符 "[image: name]"
  ↓
ANSI 文本写入 stdout
```

## 模块关系

- **上游**：操作系统 stdin（用户输入）、Gateway（调用 TerminalRenderer 渲染出站）
- **下游**：Gateway（产 NormalizedMessage 入站）、stdout（渲染结果输出）
- **与模块内其他子功能**：使用 TerminalRenderer 完成出站渲染，renderer 文档定义详细的块类型渲染规则
- **无关**：CLI Admin（Admin 命令不走消息链路，不经过 TerminalPlugin）、IM Adapter 的具体平台实现（terminal 渠道与其平级）
