# 技能清单生成

## 概述

技能清单生成机制负责将注册中心中的可用技能渲染为格式化摘要文本，供 System Prompt（SP）Builder 在组装时生成 SkillsSection，注入 SP 静态层。

## 架构

列表生成器从注册中心获取数据并格式化，输出给 SP Builder 内部的 SkillsFragmentProvider。生成器负责过滤、排序、格式化；SkillsFragmentProvider 负责读取 Session 激活状态、在 SP 重建时纳入已激活条件技能。

```
DiskSkillRegistry + BuiltinSkillRegistry  ─── 数据源
        │
        ▼
┌─ 列表生成器 ──────────────────────────┐
│  过滤：user-invocable 已声明           │
│        paths 已声明且未激活 → 排除     │
│  排序：来源优先级高→低 → 字母序        │
│  格式化：4 种变体，详见 §格式化        │
└────────────────┬──────────────────────┘
                 │
                 ▼
       SkillsFragmentProvider
       (SP Builder 组件，消费格式化结果产出 SkillsSection)
```

清单生成仅在 SP 组装时触发，组装事件之间内容不变。技能文件变更不影响当前 session 的 SP——技能集在 session 内不可变，新技能定义仅在新建会话或恢复会话时随 SP 构建加载；session 内的压缩重建同样从冻结的注册中心读取，不重扫文件系统。SP 重建时，SkillsFragmentProvider 从 Session 读取激活标记，将已激活的条件技能纳入清单一并渲染。

条件激活（paths 匹配）的增量注入由 Session 模块负责，不属于本子功能的生成范围——Session 在检测到路径匹配后，以系统消息形式在下一 turn 注入单条技能条目，无需触发 SP 重建。

### 过滤规则

初始清单仅包含声明了 `user-invocable` 的技能。声明了 `paths` 且当前 session 未激活的技能不在清单中。

SP 重建时，额外纳入当前 session 已激活的条件技能（由 Session 模块维护激活标记）。已激活技能不受 `user-invocable` 声明的限制——即使未声明 `user-invocable`，只要路径匹配触发激活即纳入重建清单。

### 排序规则

两层排序，来源优先级定义见 [skill-definition.md](skill-definition.md) 目录层级与优先级表。初始清单和 SP 重建合并清单均适用同一排序规则：
1. 按 skill 来源优先级降序（高优先级在前）
2. 同来源内按目录名（即 skill 名）字母序升序

### 格式化

根据 skill 的字段组合有四种变体，effort 字段有值时统一追加在末尾：

- 基础格式（无 when-to-use、无 paths）：
  `- **{name}**: {description}`
- 含决策提示（有 when-to-use、无 paths）：
  `- **{name}**: {description} — {when-to-use}`
- 含条件激活标记（有 paths、无 when-to-use）：
  `- **{name}**: {description} ⚡ auto-activates on: {glob patterns}`
- 含决策提示与条件激活标记（有 when-to-use、有 paths）：
  `- **{name}**: {description} — {when-to-use} ⚡ auto-activates on: {glob patterns}`

以上任一格式中，若 skill 声明了 `effort` 字段，在末尾追加 `[effort: {effort}]`。

带 ⚡ 标记的两种变体仅用于条件激活后的增量注入（由 Session 模块负责），以及 SP 重建时含已激活条件技能的完整清单。

清单为空时不注入对应段落。

## 数据流

### 清单生成

1. SP Builder 触发技能清单生成，传入当前 session 的激活标记（由 Session 模块维护）
2. 从 DiskSkillRegistry 和 BuiltinSkillRegistry 获取全部 skill 元数据
3. 过滤：
   - 基础：user-invocable 已声明 → 纳入
   - paths 已声明且当前 session 未激活 → 排除
   - SP 重建时：已激活条件技能直接纳入（不受 user-invocable 限制）
4. 排序：来源优先级降序 → 同来源内字母序升序
5. 若过滤后无条目 → 返回空，不注入 SkillsSection
6. 格式化为摘要文本 → 返回给 SP Builder
7. SP Builder 将 SkillsSection 注入 SP 静态层

### 条件激活（Session 模块负责，本模块提供格式化）

1. Agent 操作文件路径匹配某 skill 的 paths 模式（paths 为 glob 模式，详见 [skill-definition.md](skill-definition.md) §Frontmatter 字段）
2. Session 模块内部标记该 skill 为激活
3. 下一 turn，Session 模块以系统消息注入该 skill 的清单条目（含 ⚡ 标记，不含正文）
4. 调用时按需加载正文（详见 [skill-execution.md](skill-execution.md)）
5. SP 重建时，SkillsFragmentProvider 从 Session 读取激活标记，将已激活技能纳入完整清单

## 模块关系

- **上游**：DiskSkillRegistry + BuiltinSkillRegistry（数据源）、SP Builder（触发清单生成请求，传入 Session 激活标记）
- **下游**：SkillsFragmentProvider（SP Builder 内部组件，消费格式化结果，产出 SkillsSection 注入 SP 静态层）
- **无关**：processor_chain、renderer
