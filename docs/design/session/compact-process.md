# Session Compaction（会话压缩）

## 概述

Session Compaction 在会话上下文接近 LLM 上下文窗口上限时，将对话历史压缩为结构化摘要，释放 token 空间以继续对话。

Bootstrap 文件（AGENTS.md、SOUL.md、IDENTITY.md、USER.md）中包含的角色定义和行为规则通过架构隔离保护：system prompt 独立于对话消息流，不参与 compaction，因此压缩不会导致角色定义丢失或扭曲。

### 设计原则

- **Transcript 是唯一真实来源**：压缩后的 transcript 完整写入持久化存储，会话恢复时从存储加载，不依赖运行时状态。
- **幂等性**：同一对话历史多次压缩，结果一致。

## 架构

压缩从触发到持久化的链路：

```
触发 /compact 或自动阈值
  ↓
从消息历史中分离对话消息（排除 system prompt）
  ↓
[执行压缩] LLM 对对话消息做结构化摘要
  ↓
组装压缩后 transcript：system prompt + boundary message
  ↓
持久化 SessionCheckpoint
```

### 触发方式

两种触发路径，最终汇聚到同一条压缩流程：

- **手动触发**：用户输入 `/compact` 斜杠命令，可附带自定义保留指令（如 `/compact 保留用户名和邮箱`）。仅输入 `/compact` 时不附带自定义指令。
- **自动触发**：每次收到用户消息后，估算当前 token 用量，当剩余空间低于可配置阈值时自动执行压缩。自动触发内建了以下防护：

**Token 估算**：用字符数乘以配置系数做线性估算，不依赖外部 tokenizer。不同模型的上下文窗口大小在内部维护。

**分级告警阈值**：根据剩余 token 空间分为正常、预警、触发自动压缩、阻塞四个级别。阻塞状态下拒绝新请求，要求用户手动压缩。

**熔断器**：连续压缩失败达到上限后，自动压缩暂停，避免反复失败消耗资源。手动 `/compact` 不受熔断器限制，成功后熔断器自动复位。

### 压缩执行层

压缩由 LLM 完成。每次压缩时，从当前消息历史中提取对话消息（user/assistant 角色），单独发送给 LLM 做摘要。system prompt 不进入压缩消息流——LLM 只看到对话历史，不看到角色定义。

压缩专用 prompt 要求：禁止有外部副作用的工具调用，按指定维度生成结构化摘要，可选附带用户自定义保留指令。以低温度、限制输出长度的方式调用 LLM，从响应中提取摘要内容，组装为 boundary 消息。

压缩 prompt 要求 LLM 覆盖九个维度：用户身份与偏好、当前项目与上下文、关键决策与结论、未解决的问题、技术状态、对话流程、重要事实与引用、Agent 记忆与自我认知、后续步骤与行动项。

### System Prompt 隔离

system prompt 包含 bootstrap 文件内容和工具/skill 列表，在会话创建时由 Session Injection 模块组装。session 内部将 system prompt 与对话消息分开管理：

- **存储**：system prompt 作为独立字段持久化，不混入对话消息列表。
- **API 调用**：每次调用 LLM 时，system prompt 前置到消息列表最前端，与对话消息组合为完整请求。
- **Compaction**：system prompt 不进入压缩消息流。压缩后的 transcript 由 system prompt + boundary message 组成。
- **恢复**：会话从 checkpoint 恢复时，从持久化存储加载 system prompt，从 transcript 加载压缩后的对话历史。

此设计确保角色定义在任意次压缩后依然完整，无需哈希校验或重新注入。

## 数据流

### 手动压缩

```
用户输入 "/compact [可选指令]"
  → 解析命令，提取可选自定义指令
  → 从消息历史中提取对话消息（排除 system prompt）
  → 构建压缩 prompt（含自定义指令）→ 调用 LLM
  → 从响应中提取摘要 → 组装 boundary message
  → 统计 before/after 字符和 token 数
  → 用 system prompt + boundary message 替换 chat_history
  → 返回压缩统计给用户
```

失败时（LLM 调用异常、摘要解析失败）：回传错误信息，chat_history 保持不变，熔断计数器递增。

### 自动压缩

```
用户发送普通消息
  → 写入用户消息到对话历史
  → 按消息上限截断历史
  → 估算 token 数 + 熔断器检查 + 阈值判断
  → 若触发：执行压缩，用 system prompt + boundary message 替换 chat_history
  → 继续调用 LLM 处理用户消息
```

不触发或失败时：跳过压缩，继续正常对话流程。

### 压缩前后 Transcript 对比

```
压缩前（N 条消息）：
  user: 第一条消息
  assistant: 第一条回复
  user: 第二条消息
  ...

压缩后（1 条消息 + system prompt 保持不变）：
  [Session Compaction | 手动压缩] 摘要内容
```

system prompt 在压缩前后完全一致，不参与摘要流程。

## 模块关系

### 上游

- **Chat Session**：检查自动压缩阈值，截获 `/compact` 命令触发压缩。
- **Slash Command**：解析 `/compact [指令]` 格式。
- **Session Injection**：提供 system prompt，在压缩后组装 transcript 时读取。

### 下游

- **LLM Provider**：被调用来执行实际的对话摘要生成。
- **Checkpoint Manager**：压缩完成后触发 checkpoint 保存，持久化 system prompt 和压缩后的 transcript。

### 无关

- **Session Injection**（无调用关系）：仅提供 system prompt 数据，不参与压缩执行流程。
- **Archive Sweeper**（无调用关系）：管理 session 的归档和清理生命周期，与压缩流程无交互。
