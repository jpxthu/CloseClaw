# Session Compaction（会话压缩）

## 概述

Session Compaction 在会话上下文接近 LLM 上下文窗口上限时，将对话历史压缩为结构化摘要，释放 token 空间以继续对话。

Bootstrap 文件（AGENTS.md、SOUL.md、IDENTITY.md、USER.md、TOOLS.md 等）中包含的角色定义、行为规则和工具清单通过架构隔离保护：system prompt 独立于对话消息流，不参与 compaction，因此压缩不会导致角色定义丢失或扭曲。

### 设计原则

- **transcript 是唯一真实来源**：压缩后的 transcript 完整写入持久化存储，会话恢复时从存储加载，不依赖运行时状态。
- **幂等性**：同一对话历史多次压缩，结果一致。

## 架构

压缩从触发到持久化的链路：

```
触发 /compact 或自动阈值
  ↓
[创建运行快照] 压缩前由 Run Health 模块创建可回滚的 transcript 快照（详见 run-health.md）
  ↓
从消息历史中分离对话消息（排除 system prompt）
  ↓
[执行压缩] LLM 对对话消息做结构化摘要
  ↓
组装压缩后 transcript：boundary 消息
  ↓
通知 SessionManager 触发 system prompt 静态层重建（详见 session-injection.md）
  ↓
持久化 SessionCheckpoint（transcript 仅包含 boundary 消息，不含 system prompt）
```

### 触发方式

两种触发路径，最终汇聚到同一条压缩流程：

- **手动触发**：用户输入 `/compact` 斜杠命令，可附带自定义保留指令（如 `/compact 保留用户名和邮箱`）。仅输入 `/compact` 时不附带自定义指令。
- **自动触发**：每次收到用户消息后，估算当前 token 用量，当剩余空间低于可配置阈值时自动执行压缩。自动触发内建了以下防护：

**Token 估算**：组合使用服务端精确用量和字符估算。已完成的 LLM 调用使用服务端返回的精确 usage 数据（prompt_tokens、completion_tokens、cache 相关 token），从会话统计（RunningStats）中读取；尚未发送的待估消息（如用户新输入）用字符数乘以配置系数做线性估算，不依赖外部 tokenizer。两者相加得到当前上下文 token 总量。不同模型的上下文窗口大小由模型发现子系统的知识库提供。

**分级告警阈值**：根据剩余 token 空间分为正常、预警、触发自动压缩、阻塞四个级别。阻塞状态下拒绝新请求，要求用户手动压缩。

**熔断器**：连续压缩失败达到上限后，自动压缩暂停，避免反复失败消耗资源。手动 `/compact` 不受熔断器限制，成功后熔断器自动复位。

### 压缩执行层

压缩由 LLM 完成。每次压缩时，从当前消息历史中提取对话消息（user/assistant 角色），单独发送给 LLM 做摘要。system prompt 不进入压缩消息流——LLM 只看到对话历史，不看到角色定义。

压缩专用 prompt 要求：禁止有外部副作用的工具调用，按指定维度生成结构化摘要，可选附带用户自定义保留指令。以低温度、限制输出长度的方式调用 LLM，从响应中提取摘要内容，组装为 boundary 消息。

**boundary 消息**是压缩后的 transcript 中的唯一消息条目，承载 LLM 生成的对话历史结构化摘要（覆盖九个维度）及压缩元信息（压缩时间、触发方式）。压缩完成后 transcript 的对话部分只包含这一条 boundary 消息，替代压缩前的全部 user/assistant 消息。

压缩 prompt 要求 LLM 覆盖九个维度：用户身份与偏好、当前项目与上下文、关键决策与结论、未解决的问题、技术状态、对话流程、重要事实与引用、Agent 记忆与自我认知、后续步骤与行动项。

### System Prompt 隔离

system prompt 包含 bootstrap 文件内容和工具/skill 列表，在会话创建时由 Session Injection 模块组装。session 内部将 system prompt 与对话消息分开管理：

- **存储**：system prompt 作为独立字段保存在 ConversationSession 运行时对象中，与对话消息列表分开管理。
- **API 调用**：每次调用 LLM 时，system prompt 前置到消息列表最前端，与对话消息组合为完整请求。
- **Compaction**：system prompt 不进入压缩消息流。压缩后的 transcript 仅包含 boundary 消息。system prompt 由独立的注入流程管理，压缩完成后从最新 bootstrap 文件重建。
- **恢复**：会话从 checkpoint 恢复时，重新走注入流程，从工作目录加载最新 bootstrap 文件重建 system prompt，确保 prompt 内容最新。

此设计确保角色定义在任意次压缩后依然完整，无需哈希校验或重新注入。

## 数据流

### 手动压缩

```
用户输入 "/compact [可选指令]"
  → 解析命令，提取可选自定义指令
  → 从消息历史中提取对话消息（排除 system prompt）
  → 构建压缩 prompt（含自定义指令）→ 调用 LLM
  → 从响应中提取摘要 → 组装 boundary 消息
  → 统计 before token 数（已返回消息用精确 usage + 待发送消息用字符估算）
  → 用 boundary 消息替换对话消息
  → 统计 after token 数（boundary 消息用字符估算）
  → 通知 SessionManager，由 SessionManager 触发注入流程重建 system prompt 静态层
  → 持久化压缩后的 transcript（仅 boundary 消息）
  → 返回压缩统计给用户
```

失败时（LLM 调用异常、摘要解析失败）：回传错误信息，chat_history 保持不变，熔断计数器递增。

### 自动压缩

```
用户发送普通消息
  → 写入用户消息到对话历史
  → 按消息上限截断历史
  → 估算 token 数 + 熔断器检查 + 阈值判断
  → 若触发：执行压缩，用 boundary 消息替换对话消息
  → 通知 SessionManager，由 SessionManager 触发注入流程重建 system prompt 静态层
  → 持久化压缩后的 transcript（仅 boundary 消息）
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

压缩后 transcript（仅 1 条 boundary 消息）：
  [Session Compaction | 手动压缩] 摘要内容

压缩结束后，SessionManager 触发注入流程重建 system prompt 静态层。system prompt 不在 transcript 中，它是 session 独立管理的运行时字段。
```

LLM 摘要步骤不修改 system prompt 内容。Compaction 结束后，SessionManager 触发注入流程重建 system prompt 的静态层（角色定义、工具与 Skill 清单、长期记忆等 Section），确保下次 API 请求时所有静态内容反映最新配置。静态层各 Section 的构建数据源详见 [system_prompt/README](../system_prompt/README.md)。

## 模块关系

### 上游

- **Chat Session**：检查自动压缩阈值，截获 `/compact` 命令触发压缩。
- **Slash Command**：解析 `/compact [指令]` 格式。
- **LLM 模块（会话统计）**：提供已完成的 LLM 调用的精确 usage 数据（prompt_tokens、completion_tokens、cache 相关 token），用于 token 估算。
- **LLM 模块（模型发现）**：提供模型上下文窗口大小等推荐参数，用于阈值判断。

### 下游

- **LLM Provider**：被调用来执行实际的对话摘要生成。
- **Checkpoint Manager**：压缩完成后触发 checkpoint 保存，持久化压缩后的 transcript（system prompt 为运行时字段，不进入 SessionCheckpoint）。
- **Session Injection**：压缩完成后通知 SessionManager，由 SessionManager 触发注入流程重建 system prompt（间接下游，详见 [session-injection.md](session-injection.md)）。

### 无关

- **Archive Sweeper**（无调用关系）：管理 session 的归档和清理生命周期，与压缩流程无交互。
