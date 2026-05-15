# Session Compaction（会话压缩）

## 概述

Session Compaction 在会话上下文接近 LLM 上下文窗口上限时，将对话历史压缩为结构化摘要，释放 token 空间以继续对话。压缩过程通过 Bootstrap 保护机制确保角色定义、行为规则等核心内容不在摘要中丢失或扭曲。

### 设计原则

- **Transcript 是唯一真实来源**：压缩后的 transcript 完整写入持久化存储，会话恢复时从存储加载，不依赖运行时状态。
- **幂等性**：同一 session 使用同样的 bootstrap 内容多次压缩，结果一致。当 bootstrap 文件未变更时，压缩前后 transcript 中 bootstrap 区域内容保持稳定。
- **事务性**：压缩前后的保护校验与持久化串行执行，不在中间状态持久化。

## 架构

压缩从触发到持久化是一条事务性链路：

```
触发 /compact 或自动阈值
  ↓
[PreCompact] 发射事件 → 记录所有 region 内容哈希
  ↓
[执行压缩] LLM 对对话历史做结构化摘要
  ↓
[PostCompact] 发射事件 → 检测 region 完整性
  ↓
若 region 被扭曲：从 workspace 重新读取文件 → reinject block 前置到 boundary 消息之前
  ↓
替换 chat_history 为 boundary message + 可能的 reinject block
  ↓
持久化 SessionCheckpoint（含 BootstrapContext）
```

以下各小节依次展开链路中的关键机制。

### 触发方式

两种触发路径，最终汇聚到同一条压缩流程：

- **手动触发**：用户输入 `/compact` 斜杠命令，可附带自定义保留指令（如 `/compact 保留用户名和邮箱`）。
- **自动触发**：每次收到用户消息后，估算当前 token 用量，当剩余空间低于可配置阈值时自动执行压缩。自动触发内建了以下防护：

**Token 估算**：用字符数乘以配置系数做线性估算，不依赖外部 tokenizer。不同模型的上下文窗口大小在内部维护。

**分级告警阈值**：根据剩余 token 空间分为正常、预警、触发自动压缩、阻塞四个级别。阻塞状态下拒绝新请求，要求用户手动压缩。

**熔断器**：连续压缩失败达到上限后，自动压缩暂停，避免反复失败消耗资源。手动 `/compact` 不受熔断器限制，成功后熔断器自动复位。

### 压缩执行层

压缩由 LLM 完成。构造特殊 prompt（禁止有外部副作用的工具调用 + 结构化摘要要求 + 可选自定义指令），以低温度、限制输出长度的方式发送给 LLM，从响应中提取摘要内容，组装为 boundary 消息。

压缩 prompt 要求 LLM 覆盖九个维度：用户身份与偏好、当前项目与上下文、关键决策与结论、未解决的问题、技术状态、对话流程、重要事实与引用、Agent 记忆与自我认知、后续步骤与行动项。

压缩后的 transcript 结构见下文「压缩前后 Transcript 对比」。

### Bootstrap 保护机制

压缩可能导致注入在对话开头的角色定义文件（AGENTS.md、SOUL.md 等）被摘要扭曲。保护层通过 region marker + 哈希校验确保内容完整（流程见架构总图）：

- **压缩前**：记录 transcript 中所有 region 的内容哈希（SHA-256 前 12 位）。
- **压缩后**：扫描 transcript，对比哈希检测内容是否被扭曲。
- **若扭曲**：从 workspace 重新读取原文件，生成 **reinject block**（含新 marker 的 bootstrap 文件内容块），前置到 boundary 消息之前。

**Region Marker 格式**：

```
<bootstrap:file=AGENTS.md,hash=abc123def456,chars=1234,reinject=false>
## AGENTS.md
[文件内容]
</bootstrap>
```

Marker 属性说明：
- `file`：文件名
- `hash`：内容 SHA-256 前 12 位，用于完整性校验
- `chars`：原始内容字符数
- `reinject`：是否为压缩后重注入（false=原始注入，true=重注入）

### BootstrapContext 生命周期

**创建**：会话首次启动时，扫描 transcript 中的 bootstrap 内容，为每个文件创建 region 记录，汇总为 `BootstrapContext`。

**更新**：每次压缩后若触发了重新注入，新增的 reinject region 加入 `BootstrapContext`，更新总字符数。

**持久化**：`BootstrapContext` 随 `SessionCheckpoint` 一起写入持久化存储。

**恢复**：会话从 checkpoint 恢复时，从已存储的 `BootstrapContext` 加载所有 region 信息，从已有的 region marker 重建上下文，不重新包装。恢复后继续参与后续压缩保护。

## 数据流

### 手动压缩

```
用户输入 "/compact [可选指令]"
  → 解析命令，提取可选自定义指令
  → 构建压缩 prompt（含自定义指令）→ 调用 LLM
  → 从响应中提取摘要 → 组装 boundary message
  → 统计 before/after 字符和 token 数
  → 替换 chat_history 为 boundary message
  → 返回压缩统计给用户
```

失败时（LLM 调用异常、摘要解析失败、Bootstrap 校验失败）：回传错误信息，chat_history 保持不变，熔断计数器递增。

### 自动压缩

```
用户发送普通消息
  → 写入用户消息到对话历史
  → 按消息上限截断历史
  → 估算 token 数 + 熔断器检查 + 阈值判断
  → 若触发：执行压缩，替换 chat_history 为 boundary message
  → 继续调用 LLM 处理用户消息
```

不触发或失败时：跳过压缩，继续正常对话流程。

### 压缩前后 Transcript 对比

```
压缩前（N 条消息）：
  system: <bootstrap:file=AGENTS.md,reinject=false>...</bootstrap>
  user: 第一条消息
  assistant: 第一条回复
  ...

压缩后（1 条消息 + 可能的 reinject）：
  system: <bootstrap:file=AGENTS.md,reinject=true>...</bootstrap>  （仅在扭曲时前置）
  system: [Session Compaction | 手动压缩] 摘要内容
```

## 模块关系

### 上游

- **Chat Session**：检查自动压缩阈值，截获 `/compact` 命令触发压缩。
- **Slash Command**：解析 `/compact [指令]` 格式。

### 下游

- **LLM Provider**：被调用来执行实际的对话摘要生成。
- **Bootstrap Protection**：压缩前后通过完整性校验和重新注入保护 bootstrap 内容。
- **Checkpoint Manager**：压缩前后发射事件，触发 checkpoint 保存。`BootstrapContext` 随 checkpoint 持久化。

### 无关

- **Session Injection**：在会话创建时组装 system prompt，与压缩操作不同阶段的 transcript，无调用关系。
- **Archive Sweeper**：管理 session 的归档和清理生命周期，与压缩流程无交互。
