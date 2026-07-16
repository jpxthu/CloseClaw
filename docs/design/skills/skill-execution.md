# 技能调用

## 概述

技能调用机制负责 Agent 通过 SkillTool 加载并执行 skill 正文。正文统一在当前 Agent 上下文 inline 执行，不创建隔离子 Agent。

## 架构

SkillTool 统一完成技能调用流程：

1. 从注册中心（DiskSkillRegistry → 未命中则 BuiltinSkillRegistry）按 skill 名称（即目录名）查找 skill 实例
2. 按需加载 skill 正文（SKILL.md 指令文本）
3. 替换正文中的 `${SKILL_DIR}` 和 `${SESSION_ID}` 变量，未识别的 `${...}` 模式保持原样
4. 将正文注入 Agent 对话上下文

```
Agent 决策调用 skill
  ↓
SkillTool — 查找 → 加载 → 变量替换 → 注入
```

### SkillTool

Skills 模块通过 Tools 模块的注册机制向 ToolRegistry 注册 SkillTool。Agent 根据技能清单（由 skill-listing-injection 模块在每个 turn 生成并注入）中的 description 和 when-to-use 判断是否调用某个 skill，调用时通过 SkillTool 发起。

User 也可通过斜杠命令直接调用声明了 user-invocable 字段的技能。

### 正文加载

正文采用按需加载策略——注册阶段仅解析 frontmatter 并缓存元数据，skill 被调用时才从磁盘读取正文内容。正文加载完成后进行变量替换：`${SKILL_DIR}` 引用技能所在目录路径，`${SESSION_ID}` 引用当前会话 ID。

### Inline 执行

技能正文统一在当前 Agent 上下文 inline 执行。正文内容展开到 Agent 的对话上下文，Agent 按指令继续处理。不创建隔离子 Agent，不产生额外的权限隔离。

### SkillCreator

Skills 模块同时注册 SkillCreator 工具，供 Agent 在对话中创建或修改 skill 文件。Agent 根据 User 描述的需求，使用该工具生成符合规范的 SKILL.md 文件（含正确的 frontmatter 配置和指令正文）。

## 数据流

Agent 通过技能清单（由 skill-listing-injection 在每个 turn 生成并注入）获知可用技能，据此决策是否调用。调用流程：

1. Agent 决策调用 skill
2. SkillTool 收到调用请求
3. 按 skill 名称（即目录名）从 DiskSkillRegistry 查找
   - 未命中 → 查 BuiltinSkillRegistry
   - 仍不存在 → 返回错误
4. 从磁盘按需加载 skill 正文（指令文本）
5. 替换正文中的 `${SKILL_DIR}` 和 `${SESSION_ID}` 变量，未识别变量保持原样
6. 正文注入 Agent 对话上下文
7. Agent 按指令继续执行

## 模块关系

- **上游**：Agent 运行时（决策调用 skill 并触发 SkillTool）
- **下游**：DiskSkillRegistry、BuiltinSkillRegistry（查找 skill 实例、加载正文）
- **无关**：session（skill 不创建子 session）、permission（skill 不携带权限，权限由 Agent 配置管理）
- **共享类型**：无
