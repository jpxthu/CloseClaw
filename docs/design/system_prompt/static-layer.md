# 静态层

## 概述

静态层是 System Prompt 中在 Session 生命周期内保持不变的部分，在 Session 创建、归档恢复、压缩完成或 Owner 清除追加指令时重新组装，写入 ConversationSession 运行时字段，除触发重新组装事件外内容不变。

## 架构

静态层由四类 Section 组成：Bootstrap 文件 Section，以及 Tools / Skills / Memory 三个系统生成的 Section。Bootstrap 文件同样经 BootstrapFragmentProvider 产出，与其余 Section 走同一 Provider 链。

### Session 类型与 Bootstrap 文件加载

System Prompt 按 Session 类型加载内容，共三种类型：

| Session 类型 | 判定条件 | 加载内容 |
|--------------|---------|---------|
| 主 Agent Session | 顶层 Session，且未命中后两类 | 必须 Bootstrap 文件 + 可选内容（自定义引导指令仅在身份加载模式=完整时加载）+ 工具/技能清单 + MemorySection |
| 子 Session | 由主 Agent Session spawn 的子 Session，且未命中后一类 | 必须 Bootstrap 文件 + 工具/技能清单 |
| 无 Bootstrap 文件的 Session | Agent 配置显式声明无 Bootstrap 文件，或未声明但 Bootstrap 文件目录不存在 | 仅工具/技能清单 |

判定优先级：**显式声明无 Bootstrap 文件 > Bootstrap 文件目录不存在 > 主/子角色**。前两个条件任一成立即为「无 Bootstrap 文件的 Session」，与主/子角色无关（需求 [system_prompt §F8](../../requirements/system_prompt.md)（Session 类型适配））。

- Bootstrap 文件目录取自 agent 配置的 `agentDir`。目录不存在（或路径不是目录）时，即使 Agent 未声明无 Bootstrap 文件，也按「无 Bootstrap 文件的 Session」兜底，保证对话不中断，并在构建时记录一条降级告警日志（见 [debug_log §F2](../../requirements/debug_log.md)（分层日志级别））。
- 显式声明无 Bootstrap 文件属正常功能场景，不记降级告警。
- 目录存在但个别文件缺失时，仅跳过该文件，不改变 Session 类型，也不记降级告警。
- Session 类型在每次构建时判定一次；两次构建之间的目录与配置变化在下次构建时反映（见需求 §F6（内容缓存与自动刷新））。

Bootstrap 文件按文件名格式化渲染，每文件以 `## 文件名` 为标题，作为独立 Section 注入 System Prompt 前缀。多文件按固定顺序注入，AGENTS.md（操作规程）排在最高优先级（最前）。

| 文件 | 角色 | 主 Agent Session | 子 Session | 无 Bootstrap 文件的 Session |
|------|------|------------------|-----------|------------------------------|
| AGENTS.md | 操作规程 | ✅ | ✅ | ❌ |
| SOUL.md | 角色定义 | ✅ | ✅ | ❌ |
| IDENTITY.md | 身份声明 | ✅ | ✅ | ❌ |
| USER.md | Owner 信息 | ✅ | ✅ | ❌ |
| TOOLS.md | 工具使用指南 | ✅ | ✅ | ❌ |
| BOOTSTRAP.md（自定义引导指令） | 自定义引导指令 | 仅身份加载模式=完整 时 ✅ | ❌ | ❌ |
| MEMORY.md（长期记忆） | 长期记忆 | ✅ | ❌ | ❌ |
| HEARTBEAT.md | 心跳配置 | ❌ | ❌ | ❌ |

> MEMORY.md 不属于 Bootstrap 集合，列在表中仅用于对照加载范围；其注入规则见下文 MemorySection。

- 长期记忆（MEMORY.md）仅为主 Agent Session 注入，与身份加载模式无关——精简模式主 Agent Session 同样加载，只取决于 Session 类型（见 MemorySection）。
- 自定义引导指令（BOOTSTRAP.md）仅在主 Agent Session 且身份加载模式为完整时才注入；子 Session 与「无 Bootstrap 文件的 Session」恒不注入，与目标 agent 的 `bootstrapMode` 无关。
- HEARTBEAT.md 位于 Bootstrap 文件目录下，但不属于 Bootstrap 注入集合——它是周期任务触发时由 Agent 按需读取的动态上下文，不注入 System Prompt。
- Agent 的唯一 ID 与显示名称属 agent 配置档案的标识字段（见 [agent §F1](../../requirements/agent.md)（Agent 配置档案）），不属于 Bootstrap 集合，不注入 System Prompt。

### 系统生成的 Section

| Section | 内容 | 来源 |
|---------|------|------|
| ToolsSection | 当前 Agent 可用工具的分组索引（按 agent 工具白/黑名单过滤后；名称 + 危险度标记 + 常用工具的行为描述） | ToolRegistryQuery（经 AgentToolsConfigQuery 过滤） |
| SkillsSection | 技能清单（user-invocable 技能 + 当前 Session 已激活的条件技能） | SkillListingProvider（经 SkillsFragmentProvider 纳入已激活条件技能） |
| MemorySection | 跨 Session 的长期记忆 | MEMORY.md（路径见 [memory/config](../memory/config.md)） |

动态层的 ChannelContext、WorkingDirectory 两个 Section 不属于静态层，由每次请求即时构建，详见 [dynamic-layer.md](dynamic-layer.md)。

单个 Section 组装失败时跳过该 Section，其余继续。

ToolsSection 按分组聚合输出，常用工具注入完整行为描述，延迟工具仅注入名称和危险度标记。一级索引有总长度上限、超出时按分组截断。ToolsSection 实际内容与分组索引/截断规则由 ToolRegistryQuery 产出（见 [tools 模块](../tools/tools-prompt-injection.md) 工具分组索引与长度控制）。ToolsSection 为空时不注入对应段落。

#### 体积精简与截断优先级

System Prompt 总体积应保持精简（需求 NFR）。静态层的可压缩点集中在系统生成的清单类 Section——其中工具清单设有一级索引长度上限与截断（见上）。截断遵循不破坏内容完整性的优先级：**永不截断 Bootstrap 身份文件与自定义引导指令**（它们是 Agent 身份与行为准则的完整承载，必须完整注入）；超限时从系统生成、可再生的清单内容压缩（优先工具条目、再技能条目），被省略的明细通过按需查询（如工具二级注入）补足。MemorySection 的体积由 memory 模块的规则上限（`dreaming.capacity.max_rules`）约束，本模块不截断记忆内容；压缩工具与技能清单后仍超限时记录降级告警，不截断身份文件与记忆内容。此优先级仅在超限时触发；未超限轮次保持组装结果逐字节稳定，不影响前缀缓存。

MemorySection 仅为主 Agent Session 生成——子 Session 与「无 Bootstrap 文件的 Session」均不加载长期记忆，MemoryFragmentProvider 返回空 Fragment；主 Agent Session 无论身份加载模式（完整/精简）均加载，加载只取决于 Session 类型。MEMORY.md 由 memory 模块产出，路径取自 [memory/config](../memory/config.md) 的 `storage.memory_md_path`（全局规则文件，与 Session 工作目录无关）；文件缺失时该 Section 为空。详见 [fragment-provider.md](fragment-provider.md) MemoryFragmentProvider 行为。

SkillsSection 由 [SkillListingProvider](../common/core-traits.md#skilllistingprovider) 生成技能清单（过滤、排序、格式化），SkillsFragmentProvider 在 SP 重建时结合 Session 激活状态纳入已激活条件技能。三种 Session 类型均加载 SkillsSection。清单生成规则见 [skills/skill-listing-injection](../skills/skill-listing-injection.md)。技能正文不进入 System Prompt，由 skills 模块在模型决定使用时按需注入。SkillsSection 为空时不注入对应段落。

### Section 级缓存

静态层内容走 Session 级 Section 缓存。文件型 Section 基于 mtime 校验——文件未变更时命中缓存，避免重复读取和字符串拼接。ToolsSection 通过显式缓存失效触发重建。

此缓存节省本地文件读取和字符串拼接开销，与 API 侧的 KV Cache 是独立的两层优化。

缓存失效标记在下次 System Prompt 组装时生效；静态层不会在两次组装之间因数据源变更而重建——文件、工具与技能注册中心变更均在下次组装时反映（需求 §F6（内容缓存与自动刷新））。

缓存失效触发：
- Bootstrap 文件或 MEMORY.md 变更 → 对应 Section（Bootstrap 为聚合后的单 Fragment）缓存失效，下次组装时重新读取
- 技能注册中心变更（skill 文件新增/修改/删除）→ SkillsSection 缓存失效，下次组装时重建
- `/system clear` → 清空追加区 Owner 动态指令的同时，全部静态层缓存失效并触发重新组装（需求 §F5（追加指令管理））
- 工具注册中心变更（工具定义新增/修改/删除）→ ToolsSection 缓存失效，下次组装时从 ToolRegistryQuery 重新生成
- Session 恢复 → 强制重建全部静态层，确保内容与最新文件一致
- Compaction → 触发 System Prompt 重建回调，强制重建全部静态层

条件技能激活状态由 Session 以系统消息增量注入，不改变 SkillsSection；SkillsSection 缓存键不含激活状态，仅 SP 重建时纳入已激活技能。

### 兜底与变体

当所有 Section 渲染结果为空时，静态层退化为默认 prompt 单段文本（不含 Section 标题）："You are CloseClaw, a helpful AI assistant."。默认文本由 Builder 在 Provider 链全部返回空时产出。

静态层内容由 Session 类型统一决定（见 §Session 类型与 Bootstrap 文件加载）：主 Agent Session 加载必须 Bootstrap 文件、可选内容与 MemorySection；子 Session 加载必须 Bootstrap 文件；「无 Bootstrap 文件的 Session」两者都不加载。ToolsSection 与 SkillsSection 不受 Session 类型影响，三种类型均加载；为空时不注入对应段落。工作目录（workspace）不参与静态层内容判定——它只用于动态层 WorkingDirectory。

## 数据流

```
1. SessionManager 创建新 Session / 恢复归档 / 压缩完成 / Owner 清除追加指令时触发构建，传入 agent_id、Session 角色（主/子）与身份加载模式覆写（如有）；Builder 启动时已持有 Provider 列表与 ToolRegistryQuery 引用
2. Builder 经 AgentLookup 查询 agent 配置，得到 `agentDir`、`bootstrapMode` 默认值与 `noBootstrap`（有覆写时以覆写为准）；检查 Bootstrap 文件目录是否存在，解析 Session 类型（显式声明 > 目录不存在 > 主/子）。因目录不存在兜底时记录降级告警
3. 按 Session 类型通过 Bootstrap Loader 加载 Bootstrap 文件（命中缓存则跳过读取）
4. ToolRegistryQuery 生成按 agent 白/黑名单过滤后的工具分组索引
5. SkillsFragmentProvider 经 SkillListingProvider 生成技能清单，并在 SP 重建时按 `activated_skills` 纳入已激活条件技能
6. 主 Agent Session 且 MEMORY.md 存在时读取 MEMORY.md（命中缓存则跳过）；子 Session 与「无 Bootstrap 文件的 Session」跳过
7. 组装静态层：Bootstrap 文件 + ToolsSection + SkillsSection + MemorySection（按 Session 类型取舍，详见兜底与变体）
8. 写入 ConversationSession 的 System Prompt 字段（运行时字段，不进 SessionCheckpoint）
```

## 模块关系

### 上游

- **SessionManager**：在 Session 创建、归档恢复、压缩完成与 Owner 清除追加指令时触发静态层构建，传入 agent_id、Session 角色与身份加载模式覆写（如有）。
- **Agent 模块**：提供 `agentDir`、`bootstrapMode` 与 `noBootstrap` 配置，用于定位 Bootstrap 文件并判定 Session 类型。
- **Bootstrap Loader**：提供 Bootstrap 文件内容，按 Session 类型选择文件集。
- **ToolRegistryQuery**：提供经 agent 白/黑名单过滤后的 ToolsSection 分组索引。
- **SkillListingProvider**：提供过滤、排序、格式化后的技能清单。
- **Session 压缩流程（Compaction）**：压缩完成后通过回调触发静态层重建。

### 下游

- **ConversationSession**：构建完成后写入其 System Prompt 字段，每次 API 请求时取出使用。

### 无关

（无）
