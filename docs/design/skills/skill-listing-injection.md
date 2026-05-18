# Skill 列表注入

## 概述

Skill 列表注入机制负责在 session 启动时将可用 skill 的摘要清单注入到 agent 上下文中，让 agent 在对话初始化时就知道有哪些 skill 可用、何时使用。同时支持热重载替换和 compaction 保护。

核心设计：
- 纯函数生成：固定排序规则，相同输入产生相同输出
- 注入位置固定：session 最开头的 system-reminder 消息
- 热重载直接替换：不缓存、不做增量优化
- compaction 原样保留：压缩只针对多轮对话部分

## 架构

列表注入由三个子组件协作完成：列表生成器、注入器、文件监听器。

```
统一注册中心（DiskSkillRegistry + SkillRegistry）─── 数据源
        │
        ▼
┌─ 列表生成器（Listing Generator）─┐
│  过滤：agent-id 匹配             │
│        user-invocable = true    │
│  排序：source 优先级 → name 序   │
│  格式化：## Available Skills     │
│         - **{name}**: {desc}    │
│           ⚡ auto-activates on   │
└────────────────┬────────────────┘
                 │
                 ▼
┌─ 注入器（Injector）───────────┐
│  创建 system-reminder 消息       │
│  标记 is_meta=true（用户不可见） │
│  附加 SkillListingAttachment    │
│  插入 session transcript 最开头  │
└────────────────┬────────────────┘
                 │
                 ▼
┌─ 文件监听器（Skill Watcher）──┐
│  监听技能目录文件变更            │
│  300ms debounce 聚合事件        │
│  变更后触发 → 重新生成 → 替换   │
└────────────────────────────────┘
```

### 列表生成

列表生成是纯函数，输入 DiskSkillRegistry 和 agent_id，输出格式化字符串。格式根据 skill 的 whenToUse 和 paths 字段有三种变体：

```
## Available Skills
- **{name}**: {description} — {whenToUse}
  ⚡ auto-activates on: {glob pattern}
```

whenToUse 可选（无则不显示 `—` 后缀），paths 可选（无则不显示 ⚡ 行）。只含 paths 不含 whenToUse 时格式为：

```
- **{name}**: {description}
  ⚡ auto-activates on: {glob pattern}
```

排序规则两层：
1. 按 skill 来源优先级排序（高优先级在前）
2. 同来源内按 name 字母序升序

过滤条件：
- 仅注入 agent-id 匹配的 skill（agent-id 为空或匹配当前 agent）
- 仅注入 user-invocable 为 true 的 skill（用户可通过 slash command 显式调用）

### 列表注入

Session 启动时，生成的 skill listing 作为 system-reminder 类型的消息插入 session transcript 最开头。该消息标记为 is_meta=true，表示元信息消息，不在对话主流程中对用户可见。

消息附带附件（skill listing attachment），记录以下元数据供后续热重载和 compaction 定位使用：
- 格式化文本内容
- 当前 skill 总数
- 是否初始批次标记

### 热重载替换

文件监听器通过底层文件系统事件机制检测 SKILL.md 的创建、修改、删除。热重载链路跨模块协作：

1. **skills 模块负责**：文件变更事件触发 → 300ms debounce → 使注册中心 listing 缓存失效 → 重新扫描变更目录 → 重新生成 listing
2. **system prompt 构建器负责**：检测到缓存已失效 → 获取新列表 → 查找 session 转录中旧的 listing 消息 → 替换消息内容（保留消息位置）→ 更新附件元数据标记为非初始批次

### Compaction 保护

Session 压缩时将 session 消息分为三个区域处理：

- **保护区域**：bootstrap 文件（AGENTS.md、SOUL.md 等）和 skill listing 消息，原样保留
- **压缩区域**：多轮对话部分（tool calls + responses），通过摘要器压缩为摘要消息
- **拼接方式**：bootstrap + skill listing + 压缩后的对话摘要

### 条件激活

带 `paths` 字段的 skill 在 listing 中有特殊标注（⚡ 标记），表示该 skill 在操作匹配文件时自动激活。

动态激活列表是 session 运行时维护的一份临时列表，记录当前活跃上下文中因文件路径匹配而处于激活状态的 skill。列表来源：agent 操作的文件路径匹配某 skill 的 paths glob → 该 skill 加入动态激活列表 → 在下一轮 listing 中体现。compaction 时动态激活列表随 listing 消息原样保留。

## 数据流

### 启动注入

```
Session 启动
  │
  ├─ 加载 bootstrap 文件
  │
  ├─ 从 DiskSkillRegistry 获取全部 skill
  │     │
  │     ├─ agent_id 匹配？（不匹配 → 跳过）
  │     └─ user_invocable 匹配？（false → 跳过）
  │
  ├─ 排序：source 优先级高→低
  │         └─ 同 source 内 name 字母序
  │
  ├─ 格式化：## Available Skills
  │         - **{name}**: {description} — {whenToUse}
  │           ⚡ auto-activates on: {paths}
  │
  └─ 创建 system-reminder 消息（is_meta=true）
        └─ 插入 session transcript 最开头
            └─ agent 在对话初始化时读取全部可用 skill
```

### 热重载

```
SKILL.md 文件变更
  → 文件监听器捕获事件
    → 300ms debounce 等待写入完成
      → 使注册中心缓存失效
        → 重新扫描变更目录
          → 重新生成 skill listing
            → 查找 session 开头旧 listing 消息
              → 替换消息内容
                → 更新附件元数据
                  → agent 在下一轮对话看到更新后的 skill 列表
```

### 技能调用衔接

```
Agent 读取 skill listing（摘要）
  │
  ├─ 判断：whenToUse 条件满足？
  │     ├─ 不满足 → 不调用（纯 awareness）
  │     └─ 满足 → 决策调用
  │
  ├─ 通过 SkillTool 发起调用
  │     └─ 从注册中心查找 skill 实例
  │
  ├─ 按需加载 skill 正文（SKILL.md 指令文本）
  │
  └─ 正文注入对话上下文 → agent 继续执行

此流程与列表注入完全独立：列表注入发生在 session 启动，
正文注入发生在每次调用时。
```

### Compaction 处理

```
触发 compaction
  │
  ├─ 识别 session transcript 消息结构
  │     ├─ [0..N]   bootstrap 区域（AGENTS.md 等）→ 保护
  │     ├─ [N+1]    skill listing 消息           → 保护
  │     └─ [N+2..]  多轮对话区域                  → 压缩
  │
  ├─ 压缩对话区域 → 摘要器生成摘要消息
  │
  └─ 拼接结果写入 session transcript：
        [bootstrap] + [skill listing] + [对话摘要]
```

## 模块关系

- **上游**：session 启动流程（触发初始注入）、文件监听器（触发热重载）、compaction 流程（触发保护逻辑）
- **下游**：DiskSkillRegistry（获取 skill 列表数据源）、session transcript（读取和修改消息列表）
- **相关**：Skills 模块主体（SkillTool 调用后注入正文，与 listing 注入完全独立、互不干扰）
