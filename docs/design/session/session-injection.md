# Session 注入

## 概述

描述 session 生命周期中，系统何时触发 system prompt 的构建和注入。注入是 session 生命周期事件——它决定**何时调用** System Prompt Builder，但不定义 system prompt 的结构、Section 类型和组装规则。那些定义在 [system_prompt/README](docs/design/system_prompt/README.md)。

## 架构

### 注入触发点

注入由 SessionManager 在以下时机触发，调用 System Prompt Builder：

- **新 session 创建**：会话查找与创建中检测到无匹配 session 时触发
- **archived session 恢复**：从存储恢复后触发重建（checkpoint 不存 system prompt，恢复时从最新文件重新构建）
- **compaction 完成**：压缩对话历史后触发重建，确保 system prompt 内容与最新 bootstrap 文件一致

构建逻辑、Section 类型定义、组装规则和容错策略全部在 [system_prompt/README](docs/design/system_prompt/README.md) 定义。注入链路只负责在正确的时机触发调用、传递参数、接收结果并存入 ConversationSession。

### 三分支决策

SessionManager 在会话查找与创建中对注入的处理分为三个分支：

- **命中 active session**：直接返回已有 session，不触发注入。已有 session 的 system prompt 保持不变。
- **命中 archived session**：从 SessionCheckpoint 恢复 ConversationSession 后，触发完整注入流程——与"新 session"分支相同的 builder 调用，用新构建的 system prompt 替换空值。
- **新 session**：触发完整注入流程。builder 内部通过 Bootstrap Loader 加载 bootstrap 文件，注入完成后 system prompt 存入 ConversationSession。

注入链路的参数契约：
- 入参：agent_id、ToolRegistry 引用、DiskSkillRegistry 引用
- bootstrap 文件由 builder 内部通过 Bootstrap Loader 加载，不经过注入链路传递
- 出参：组装完成的 system prompt 文本
- 结果存储：ConversationSession 的 system prompt 字段（运行时字段，不进 SessionCheckpoint）

### 每次 API 请求的动态层注入

动态层的注入时机和机制与 session 创建时的注入不同——它不经过 System Prompt Builder，而是每次 API 请求时由 ConversationSession 直接构建。Session 持有工作目录等运行时数据，Gateway 提供入站上下文（平台、会话名、发送者 ID、时间戳），动态层在拼接完整 system prompt 时即时生成，不持久化。

AppendSection 是独立于动态层的第三分区（详见 system_prompt/README 架构），由 ConversationSession 从自身运行时字段读取并拼接到 system prompt 末尾。

动态层的 Section 类型和拼接规则在 [system_prompt/README 动态层](docs/design/system_prompt/README.md#动态层) 定义。

### Session 恢复时的注入

Archived session 被重新访问时触发完整重建，流程与"新 session"分支相同。恢复时 builder 的行为详见 [system_prompt/README 恢复](docs/design/system_prompt/README.md#恢复)。

### Skill 热更新与注入时序

skill 文件变更时，DiskSkillRegistry 通过文件监听器自动更新内部缓存。下次注入触发时（新 session、archive 恢复、compaction），builder 从 registry 获取最新 listing。热更新机制的完整设计见 [skills/README](docs/design/skills/README.md)。

skill listing 直接拼接在 system prompt 中（非独立消息），因此不依赖消息 ID 定位。

### 消息级注入：memory_injection 槽位

session 暴露一个 `memory_injection` 槽位，供 memory 模块的 active-searcher 写入记忆摘要。与 system prompt 注入（构建时一次性）不同，memory_injection 是每条消息级别的、由外部写入的轻量注入通道。

**槽位结构**：
- 内容：tool role 消息文本（active-searcher 产出的浓缩记忆摘要）
- 位置模式：`AfterCurrent`（紧随当前用户消息插入）或 `BeforeNext`（排队到下一轮消息前插入）
- 生命周期：写入 → 下一轮消费 → 清空（一次性消费，不持久化）

**触发与写入**：
- 用户消息触发 active-searcher → active-searcher 写槽位，模式 = `AfterCurrent`
- agent 消息触发 active-searcher → active-searcher 写槽位，模式 = `BeforeNext`

**消费流程**（每次 API 请求组装消息时）：

```
1. 取消息历史
2. 新消息到达（用户消息 / 工具返回）
3. 检查 memory_injection 槽位，读取摘要内容和位置模式
4. 追加新消息到消息列表
5. 按模式插入摘要：
   ├── BeforeNext → 摘要插入消息列表（新消息之前）
   └── AfterCurrent → 摘要插入消息列表（新消息之后）
6. 清空槽位
7. 发送给 LLM
```

```
示例（AfterCurrent，用户消息触发搜索）：
  消息列表: [msg1, msg2, 用户: "查一下sqlite配置"]
  组装后:   [msg1, msg2, 用户: "查一下sqlite配置", tool: memory摘要]

示例（BeforeNext，agent 消息触发搜索，排队到下一轮）：
  消息列表: [msg1, msg2, tool: memory摘要, 用户: "继续"]
  组装后:   [msg1, msg2, tool: memory摘要, 用户: "继续"]
```

**与通用后台消息队列的关系**：memory_injection 是独立槽位，不走通用优先级队列（now/next/later），因为：
- 位置语义不同：通用队列只管"何时"注入，不管"在谁前面还是后面"
- 两者可共存：通用队列的消息与 memory 摘要可出现在同一批次消息中，互不冲突
- 去重：active-searcher 自行维护 per-session"已注入条目 ID 集合"，session 层的"同一任务只注入一次"去重作为额外保护层

## 数据流

### System Prompt 注入流程

```
触发条件（新 session / archive 恢复 / compaction）
  →
  SessionManager 调用 System Prompt Builder
    ├─ 传参：agent_id
    ├─ 传参：ToolRegistry 引用
    ├─ 传参：DiskSkillRegistry 引用
    └─ 返回：组装完成的 system prompt 文本
  →
  写入 ConversationSession 的 system prompt 字段
  →
  返回 session 给调用方
```

builder 内部通过 Bootstrap Loader 加载文件、按需组装各 Section。
```

注入链路不关心 builder 内部的 Section 组装细节——哪些 Section 参与、如何渲染、缺失文件如何容错、失败时如何处理——这些由 System Prompt Builder 在 [system_prompt/README 架构](docs/design/system_prompt/README.md#架构) 中定义的规则决定。注入链路只关心：触发条件、传什么参数、拿回什么结果、结果存哪里。

### 无 Workspace 时

session 无对应 workspace 目录时，builder 检测到 workspace 不存在后自行跳过 bootstrap 文件加载，仅生成 ToolsSection 和 SkillListingSection（详见 [system_prompt/README 无 Workspace 的 Session](docs/design/system_prompt/README.md#无-workspace-的-session)）。注入链路不参与此决策。

## 模块关系

### 上游

- **SessionManager**：session 生命周期协调者，在合适的时机触发 system prompt 注入。持有 ToolRegistry 和 DiskSkillRegistry 引用，作为注入参数传递给 builder。
- **Memory 模块（active-searcher）**：写入 `memory_injection` 槽位，提供 tool role 记忆摘要及位置模式。

### 下游

- **System Prompt Builder**：接收 bootstrap 文件、registry 引用和 agent_id，返回组装完成的 system prompt。builder 的内部逻辑在 [system_prompt/README](docs/design/system_prompt/README.md) 定义。

### 无关

- **LLM Provider**（无调用关系）：注入只负责把 system prompt 存入 ConversationSession，由后续的 API 请求链路取出并传递给 LLM provider。
- **Compaction 模块**（间接关联）：compaction 完成后触发注入重建，但注入链路不参与压缩逻辑。
