# Skill 列表注入

## 概述

Skill 列表注入机制负责在 session 启动时将可用 skill 的摘要清单注入到 system prompt 中，让 agent 在对话初始化时就知道有哪些 skill 可用。

核心设计：
- Skill 列表作为 `SkillListingSection` 进入 system prompt 静态层
- 列表为空时不添加此 Section
- 文件变更时通过缓存失效触发重建，不操作 session transcript

## 架构

列表注入由两个子组件协作完成：列表生成器、文件监听器。

```
统一注册中心（SkillRegistry：DiskSkillRegistry 管磁盘 skill + SkillRegistry 管内置 skill）─── 数据源
        │
        ▼
┌─ 列表生成器（generate_listing）────────┐
│  过滤：agent-id 匹配                   │
│  排序：source 优先级 → name 字母序     │
│  格式化：- **{name}**: {description}   │
│          可选 — {when_to_use}          │
│          可选 ⚡ auto-activates on      │
└────────────────┬───────────────────────┘
                 │
                 ▼
┌─ System Prompt 构建器 ─────────────────┐
│  创建 SkillListingSection(listing)     │
│  拼接进 system prompt 静态层           │
│  写入 ConversationSession              │
└────────────────┬───────────────────────┘
                 │
                 ▼
┌─ 文件监听器（Skill Watcher）──────────┐
│  监听技能目录文件变更                  │
│  300ms debounce 聚合事件               │
│  重新扫描 → 使 listing 缓存失效        │
│  下次请求重建 SkillListingSection      │
└────────────────────────────────────────┘
```

### 列表生成

列表生成由 `DiskSkillRegistry.generate_listing(agent_id)` 实现，输入注册中心和 agent_id，输出格式化字符串。格式根据 skill 的 `when_to_use` 和 `paths` 字段有三种变体：

```
- **{name}**: {description}
  基础格式（无 when_to_use、无 paths）

- **{name}**: {description} — {when_to_use}
  含决策提示（无 paths）

- **{name}**: {description} — {when_to_use} ⚡ auto-activates on: {glob patterns}
  含决策提示 + 条件激活标记
```

排序规则两层：
1. 按 skill 来源优先级排序（高优先级在前）
2. 同来源内按 name 字母序升序

过滤条件：仅注入 agent-id 匹配的 skill（agent-id 为空或匹配当前 agent）。

### 列表注入

Session 启动时，`build_from_workspace` 从 `DiskSkillRegistry` 获取 listing 内容，创建 `SkillListingSection` 加入 Sections 队列，拼接到 system prompt 字符串中。listing 为空时不添加此 Section。

SkillListingSection 渲染格式为 `## Available Skills\n\n{content}\n`。

### 热重载

热重载通过缓存失效实现，不涉及 session transcript 操作：

1. **文件监听器**：SKILL.md 文件的创建、修改、删除事件触发 → 300ms debounce → 使 `skill_listing` Section 缓存失效
2. **下次请求时**：`build_from_workspace` 或缓存失效后的 `build_system_prompt` 检测到缓存已失效 → 重新从 `DiskSkillRegistry` 获取 listing → 生成新的 `SkillListingSection`

替换在 system prompt 字符串层面发生，无需定位或修改 session transcript 中的消息。

### Compaction 保护

System prompt 独立于对话消息流存储（作为 `ConversationSession.system_prompt` 字段），compaction 只压缩对话消息列表。SkillListingSection 作为 system prompt 的一部分，天然不受 compaction 影响，无需额外保护逻辑。

### 条件激活

带 `paths` 字段的 skill 在 listing 中有特殊标注（⚡ 标记），表示该 skill 在操作匹配文件时自动激活。

条件激活的判断由 agent 在对话中根据当前操作的路径自行匹配，不依赖额外的运行时激活列表。

## 数据流

### 启动注入

```
Session 启动
  │
  ├─ 加载 bootstrap 文件
  │
  ├─ 从 DiskSkillRegistry 获取全部 skill
  │     │
  │     └─ agent_id 匹配？（不匹配 → 跳过）
  │
  ├─ 排序：source 优先级高→低
  │         └─ 同 source 内 name 字母序
  │
  ├─ 格式化：- **{name}**: {description} [— {when_to_use}] [⚡ ...]
  │
  └─ 创建 SkillListingSection → 拼接到 system prompt 静态层
        └─ agent 在对话初始化时读取全部可用 skill
```

### 热重载

```
SKILL.md 文件变更
  → 文件监听器捕获事件
    → 300ms debounce 等待写入完成
      → 使 skill_listing 缓存失效
        → 重新扫描变更目录，重建注册中心 listing 缓存
          → 下次 API 请求时 system prompt 构建检测缓存失效
            → 重新生成 SkillListingSection
              → 新的 system prompt 替换旧的
```

### 技能调用衔接

```
Agent 读取 system prompt 中的 skill listing
  │
  ├─ 判断：when_to_use 条件满足？
  │     ├─ 不满足 → 不调用
  │     └─ 满足 → 决策调用
  │
  ├─ 通过 SkillTool 发起调用
  │     └─ 从注册中心查找 skill 实例
  │
  ├─ 按需加载 skill 正文（SKILL.md 指令文本）
  │
  └─ 正文注入对话上下文 → agent 继续执行
```

列表注入发生在 session 启动时的 system prompt 构建，正文注入发生在每次 skill 调用时，两者完全独立。

## 模块关系

- **上游**：Session 启动流程（触发初始注入）、文件监听器 Skill Watcher（触发热重载）
- **下游**：DiskSkillRegistry（获取 skill 列表数据源）、System Prompt 构建器（接收 listing 字符串作为输入，创建 SkillListingSection 并拼入 system prompt）
- **相关**：Skills 模块主体（SkillTool 调用后注入正文，与列表注入完全独立、互不干扰）
- **无关**：Compaction 模块（skill listing 作为 system prompt 的一部分，天然不受压缩影响）
