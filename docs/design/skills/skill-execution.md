# 技能调用

## 概述

技能调用机制负责 Agent 通过 SkillTool 加载并执行 skill。磁盘技能和 Bundled 技能采用不同的执行模型：磁盘技能加载正文 inline 展开到 Agent 上下文；Bundled 技能通过原生 trait 方法分发执行，返回结果作为 meta message 注入。

## 架构

SkillTool 统一完成技能调用流程。按 skill 类型分流：

```
Agent 决策调用 skill
  ↓
SkillTool — 查找 → 按类型分流
  ├── 磁盘技能 → 加载正文 → 变量替换 → inline 注入
  └── Bundled 技能 → trait 方法分发执行 → meta message 注入
```

### SkillTool

Skills 模块通过 Tools 模块的注册机制向 ToolRegistry 注册 SkillTool。Agent 根据 System Prompt 中 SkillsSection 的技能清单（由 skill-listing-injection 模块在 SP 组装时生成）中的 description 和 when-to-use 判断是否调用某个 skill，调用时通过 SkillTool 发起。

User 也可通过斜杠命令直接调用声明了 user-invocable 字段的技能。

### 正文加载（磁盘技能）

磁盘技能的正文采用按需加载策略（详见 [skill-definition.md](skill-definition.md) §磁盘加载）。正文加载完成后进行变量替换，规则见 [skill-definition.md](skill-definition.md) §Frontmatter 字段。

### Inline 执行（磁盘技能）

磁盘技能正文在当前 Agent 上下文 inline 执行。正文内容展开到 Agent 的对话上下文，Agent 按指令继续处理。不创建隔离子 Agent，不产生额外的权限隔离。

### 原生执行（Bundled 技能）

Bundled 技能（如 FileOpsSkill、GitOpsSkill、SearchSkill 等）是编译期内置的系统能力，无法以纯 prompt 文本表达。调用时通过 Rust trait 方法分发执行原生代码逻辑，执行结果以 meta message 注入 Agent 对话上下文。Bundled 技能不经过正文加载和变量替换步骤。

### SkillCreator 技能

Skills 模块提供 SkillCreator 内置技能（Bundled 类型），为 Agent 提供创建技能文件的指导。调用时通过 trait 方法分发执行，返回的 meta message 中包含创建 SKILL.md 的指令指导。Agent 读取这些指令后，使用自身的文件写入能力按指导创建符合规范的 SKILL.md 文件（含正确的 frontmatter 配置和指令正文）。

## 数据流

Agent 通过 System Prompt 中的技能清单（由 skill-listing-injection 在 SP 组装时生成）获知可用技能，据此决策是否调用。调用流程：

1. Agent 决策调用 skill，或 User 通过斜杠命令直接调用
2. SkillTool 收到调用请求
3. 按 skill 名称（即目录名）从 DiskSkillRegistry 查找
   - 命中 → 磁盘技能，走步骤 4-6
   - 未命中 → 查 BuiltinSkillRegistry
     - 命中 → Bundled 技能，走步骤 7-8
     - 仍不存在 → 返回错误
4. 从磁盘按需加载 skill 正文（指令文本）
5. 替换正文中的变量（规则见 [skill-definition.md](skill-definition.md) §Frontmatter 字段），未识别变量保持原样
6. 正文注入 Agent 对话上下文
7. 通过 trait 方法分发执行原生代码逻辑
8. 执行结果以 meta message 注入 Agent 对话上下文
9. Agent 按指令或返回结果继续执行

## 模块关系

- **上游**：Agent 运行时（Agent-in-Session 决策调用 skill 并触发 SkillTool）
- **下游**：DiskSkillRegistry、BuiltinSkillRegistry（查找 skill 实例、磁盘技能加载正文、Bundled 技能 trait 方法执行）
- **无关**：session（skill 不创建子 session）、permission（skill 不携带权限，权限由 Agent 配置管理）
- **共享类型**：无
