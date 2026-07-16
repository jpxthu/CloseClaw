# 技能清单生成

## 概述

技能清单生成机制负责将注册中心中的可用技能渲染为格式化摘要文本，供 Session 模块在每个 turn 作为 attachment 注入 Agent 对话上下文。清单内容随技能文件变更自动更新。

## 架构

清单生成由列表生成器和文件监听器两个子组件协作完成。列表生成器从注册中心获取数据并格式化；文件监听器独立运行，检测技能目录文件变更后触发缓存失效。

```
DiskSkillRegistry + BuiltinSkillRegistry ─── 数据源
        │
        ▼
┌─ 列表生成器 ──────────────────────────┐
│  过滤：user-invocable 已声明           │
│        paths 已声明 → 不在初始清单     │
│  排序：来源优先级高→低 → 字母序        │
│  格式化：- **{name}**: {description}   │
└────────────────┬──────────────────────┘
                 │
                 ▼
┌─ Session 模块 ────────────────────────┐
│  技能清单作为 per-turn attachment     │
│  注入当前 turn 的 instruction block   │
│  对话压缩时受 Session 模块保护        │
│                                       │
│  负责：路径匹配检测、增量 diff 计算    │
└──────────────────────────────────────┘

┌─ 文件监听器（独立运行）───────────────┐
│  监听技能目录文件变更                  │
│  300ms debounce                       │
│  使 listing 缓存失效                   │
└──────────────────────────────────────┘
```

### 过滤规则

初始清单仅包含声明了 `user-invocable` 的技能。声明了 `paths` 的技能不在初始清单中（即使同时声明了 `user-invocable`），遵循条件激活规则。

### 排序规则

两层排序，来源优先级定义见 [skill-definition.md](skill-definition.md) 目录层级与优先级表：
1. 按 skill 来源优先级降序（高优先级在前）
2. 同来源内按目录名（即 skill 名）字母序升序

### 格式化

根据 skill 的字段组合有三种变体，effort 字段有值时统一追加在末尾：

- 基础格式（无 when-to-use、无 paths）：
  `- **{name}**: {description}`
- 含决策提示（有 when-to-use、无 paths）：
  `- **{name}**: {description} — {when-to-use}`
- 含决策提示与条件激活标记（有 when-to-use、有 paths）：
  `- **{name}**: {description} — {when-to-use} ⚡ auto-activates on: {glob patterns}`

以上任一格式中，若 skill 声明了 `effort` 字段，在末尾追加 `[effort: {effort}]`。

带 ⚡ 标记的第三种变体仅用于条件激活后的增量注入，不出现在初始清单中。

清单为空时不生成，Session 模块不注入空 attachment。

### 增量更新

增量注入由 Session 模块负责——Session 保持上一 turn 的清单状态，收到新清单后计算 diff，仅将变更条目作为 attachment 注入。增量注入保持与初始注入相同的格式与位置，确保不破坏 Agent 对话上下文的连续性。

文件变更触发的增量更新和路径匹配触发的条件激活在同一个 turn 可能同时发生。Session 模块合并两种来源的增量后统一注入，处理顺序为：先更新文件变更引起的增量，再处理条件激活的增量。

### 文件监听与热重载

技能文件变更通过文件监听器触发缓存失效，session 下一 turn 重建清单 attachment 时从注册中心获取最新数据。

流程：SKILL.md 创建/修改/删除 → 300ms debounce → 使 listing 缓存失效 → 重新扫描变更目录，更新注册中心 listing 缓存 → 下一 turn 更新附件内容。

不涉及 session transcript 操作，无需查找或修改 transcript 中的消息。

### 条件激活

声明了 `paths` 字段的技能在条件激活后，清单条目中带有特殊标注（⚡ 标记）。由 Session 模块检测 Agent 当前操作的文件路径是否匹配某技能的 `paths` 模式——匹配时 Session 模块内部维护激活标记，在下一 turn 以增量方式注入清单条目。激活标记的生命周期跟随当前 session，session 结束时清空。条件激活的增量注入仅含清单条目（不含正文），正文在调用时按需加载，详见 [skill-execution.md](skill-execution.md)。

## 数据流

### 初始清单

1. Session 启动
2. 从 DiskSkillRegistry 和 BuiltinSkillRegistry 获取全部 skill 元数据
3. 过滤：user-invocable 已声明
   - paths 已声明 → 排除（条件激活）
4. 排序：来源优先级降序 → 同来源内字母序升序
5. 格式化为摘要文本 → 返回给 Session 模块
6. Session 模块将其作为 per-turn attachment 注入 instruction block

### 增量更新

1. 技能文件变更
2. 文件监听器捕获事件
3. 300ms debounce
4. 使 listing 缓存失效
5. 重新扫描变更目录
6. 下一 turn，Session 模块请求最新清单
7. Session 模块对比上一 turn 的清单，计算增量
8. 仅注入变化条目

### 条件激活

1. Agent 操作文件路径匹配某 skill 的 paths 模式
2. Session 模块内部标记该 skill 为激活
3. 下一 turn 增量注入该 skill 的清单条目
4. Agent 看到后可在后续 turn 调用该 skill
5. 调用时按需加载正文（详见 [skill-execution.md](skill-execution.md)）

## 模块关系

- **上游**：DiskSkillRegistry + BuiltinSkillRegistry（数据源）、Session 模块（触发清单请求；对话压缩时保护技能清单免于被压缩，见 [session](../session/README.md)）
- **下游**：无（清单文本由 Session 模块消费，不属于本模块的下游调用）
- **无关**：system_prompt（技能清单不进入 system prompt 静态层）、processor_chain、renderer
