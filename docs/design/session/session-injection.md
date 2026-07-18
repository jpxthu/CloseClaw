# Session 注入

## 概述

描述 session 生命周期中的四类注入机制：system prompt 注入（构建时触发）、动态层注入（每请求即时构建）、skill 清单注入（per-turn attachment）、消息级注入（memory_injection 槽位）。此外，后台任务和子 Agent 完成后的结果注入通过优先级消息队列实现，详见 [session-execution](docs/design/session/session-execution.md#后台结果注入)。system prompt 的结构定义在 [system_prompt/README](docs/design/system_prompt/README.md)，skill 清单的生成规则定义在 [skills/skill-listing-injection](docs/design/skills/skill-listing-injection.md)。

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
- 入参：agent_id、ToolRegistry 引用
- bootstrap 文件由 builder 内部通过 Bootstrap Loader 加载，不经过注入链路传递
- 出参：组装完成的 system prompt 文本
- 结果存储：ConversationSession 的 system prompt 字段（运行时字段，不进 SessionCheckpoint）

### 每次 API 请求的动态层注入

动态层的注入时机和机制与 session 创建时的注入不同——它不经过 System Prompt Builder，而是每次 API 请求时由 ConversationSession 直接构建。Session 持有工作目录等运行时数据，Gateway 提供入站上下文（平台、会话名、发送者 ID、时间戳），动态层在拼接完整 system prompt 时即时生成，不持久化。

AppendSection 是独立于动态层的第三分区（详见 system_prompt/README 架构），由 ConversationSession 从自身运行时字段读取并拼接到 system prompt 末尾。

动态层的 Section 类型和拼接规则在 [system_prompt/README 动态层](docs/design/system_prompt/README.md#动态层) 定义。

### Session 恢复时的注入

Archived session 被重新访问时触发完整重建，流程与新 session 相同。详见 [system_prompt/README 恢复](docs/design/system_prompt/README.md#恢复)。

### Skill 清单注入（per-turn attachment）

技能清单不在 system prompt 静态层中，而是由 Session 模块在每个 turn 作为 attachment 注入 Agent 对话上下文。注入时机为每次 API 请求组装消息时、LLM 调用之前。

**清单获取**：Session 模块每 turn 从 DiskSkillRegistry 获取当前所有可见 skill 的摘要清单。清单的过滤、排序、格式化规则见 [skills/skill-listing-injection](docs/design/skills/skill-listing-injection.md)。

**注入策略**：
- **首 turn**：注入完整初始清单。清单作为 attachment 消息插入 instruction block
- **后续 turn**：Session 保持上一 turn 的清单快照，获取最新清单后计算差异（新增/删除/修改的条目），仅将变更条目作为 attachment 注入。增量注入保持与初始注入相同的格式和位置
- **空清单**：清单为空时不注入 attachment，Agent 对话上下文中不出现技能清单相关内容

**条件激活**：声明了 `paths` 匹配模式的 skill 不在初始清单中。当 Agent 当前 turn 操作的文件路径匹配某 skill 的 `paths` 模式时，Session 模块内部标记该 skill 为激活——下一 turn 以增量方式注入该 skill 的清单条目（含 ⚡ 标记，不含正文）。激活标记跟随当前 session 生命周期，session 结束时清空。

**文件变更**：skill 文件变更由 DiskSkillRegistry 的文件监听器处理（300ms debounce → 缓存失效 → 重新扫描）。Session 模块不参与监听——仅在下 turn 请求清单时获得最新数据，diff 计算自然体现变更。详见 [skills/skill-listing-injection](docs/design/skills/skill-listing-injection.md) 增量更新节。

**压缩保护**：技能清单的 attachment 消息受 Session 模块保护，不参与 compaction。详见 [skills/skill-listing-injection](docs/design/skills/skill-listing-injection.md) 模块关系节。

**与 system prompt 的关系**：技能清单不进入 system prompt 任何分区（静态层、动态层、追加区）。skill 正文在 Agent 调用时才按需加载（详见 [skills/skill-execution](docs/design/skills/skill-execution.md)），也不进入 system prompt。

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
  消息列表: [msg1, msg2, 用户: "继续"]
  组装后:   [msg1, msg2, tool: memory摘要, 用户: "继续"]
```

**与通用后台消息队列的关系**：memory_injection 是独立槽位，不走通用优先级队列（now/next/later），因为：
- 位置语义不同：通用队列只管"何时"注入，不管"在谁前面还是后面"
- 两者可共存：通用队列的消息与 memory 摘要可出现在同一批次消息中，互不冲突
- 去重：active-searcher 自行维护 per-session"已注入条目 ID 集合"，session 层的"同一任务只注入一次"去重作为额外保护层

## 数据流

### System Prompt 注入流程

1. 触发条件（新 session / archive 恢复 / compaction）
2. SessionManager 调用 System Prompt Builder
   - 传参：agent_id
   - 传参：ToolRegistry 引用
   - 返回：组装完成的 system prompt 文本
3. 写入 ConversationSession 的 system prompt 字段
4. 返回 session 给调用方

builder 内部通过 Bootstrap Loader 加载文件、按需组装各 Section。

注入链路不关心 builder 内部的 Section 组装细节——哪些 Section 参与、如何渲染、缺失文件如何容错、失败时如何处理——这些由 System Prompt Builder 在 [system_prompt/README 架构](docs/design/system_prompt/README.md#架构) 中定义的规则决定。注入链路只关心：触发条件、传什么参数、拿回什么结果、结果存哪里。

### 无 Workspace 时

session 无对应 workspace 目录时，builder 检测到 workspace 不存在后自行跳过 bootstrap 文件加载，仅生成 ToolsSection（详见 [system_prompt/README 无 Workspace 的 Session](docs/design/system_prompt/README.md#无-workspace-的-session)）。注入链路不参与此决策。

## 模块关系

### 上游

- **SessionManager**：session 生命周期协调者，在合适的时机触发 system prompt 注入。持有 ToolRegistry 引用，作为注入参数传递给 builder。同时持有 DiskSkillRegistry 引用，在每 turn 组装消息时获取 skill 清单用于 per-turn attachment 注入。
- **Memory 模块（active-searcher）**：写入 `memory_injection` 槽位，提供 tool role 记忆摘要及位置模式。

### 下游

- **System Prompt Builder**：接收 bootstrap 文件、ToolRegistry 引用和 agent_id，返回组装完成的 system prompt。builder 的内部逻辑在 [system_prompt/README](docs/design/system_prompt/README.md) 定义。
- **DiskSkillRegistry**：Session 每 turn 从此获取最新 skill 清单，用于 per-turn attachment 注入。清单过滤、排序、格式化规则在 [skills/skill-listing-injection](docs/design/skills/skill-listing-injection.md) 定义。
- **Skills 模块**：提供 skill 定义（[skills/skill-definition](docs/design/skills/skill-definition.md)）和清单生成（[skills/skill-listing-injection](docs/design/skills/skill-listing-injection.md)），Session 负责消费清单并注入为 per-turn attachment。

### 无关

- **LLM Provider**（无调用关系）：注入只负责把 system prompt 存入 ConversationSession，由后续的 API 请求链路取出并传递给 LLM provider。
- **Compaction 模块**（间接关联）：compaction 完成后触发注入重建，但注入链路不参与压缩逻辑。
