# Skill 定义

## 概述

Skill 由 SKILL.md 文件定义，包含配置清单（frontmatter）和正文（指令文本）两部分。正文指 frontmatter 之后的 Markdown 内容，是 Skill 的核心指令。

## 架构

### Frontmatter 字段

每个 SKILL.md 文件通过 frontmatter 声明以下配置：

| 字段 | 必填 | 作用 |
|------|------|------|
| description | 是 | 技能简介，供 Agent 初步判断用途 |
| when-to-use | 否 | 决策提示，帮助 Agent 判断调用时机 |
| paths | 否 | 条件激活的文件 glob 匹配模式。声明此字段的技能遵循条件激活规则（详见 [skill-listing-injection.md](skill-listing-injection.md) §条件激活） |
| user-invocable | 否 | 声明后 User 可通过斜杠命令直接调用。无 paths 时出现在技能清单中；有 paths 时遵循条件激活规则（不在初始清单中，详见 [skill-listing-injection.md](skill-listing-injection.md) §条件激活），但仍可通过斜杠命令调用。不声明 user-invocable 则不出现清单，也不可通过斜杠命令调用 |
| effort | 否 | 成本估算，供 Agent 调度参考 |

正文支持变量替换：`${SKILL_DIR}` 引用技能所在目录路径，`${SESSION_ID}` 引用当前会话 ID。

技能仅提供纯 prompt 指令，不携带任何工具权限。工具权限由 Agent 配置统一管理。

### 目录层级与优先级

五个技能来源按优先级从高到低排列：

| 层级 | 路径 | 作用域 | 优先级 |
|------|------|--------|--------|
| Project | `<project-root>/.closeclaw/skills/` | 仅该项目 | 最高 |
| Agent | `~/.closeclaw/agents/<agent-id>/skills/` | 仅该 Agent | 高 |
| Global | `~/.closeclaw/skills/` | 所有 Agent | 中 |
| ExtraDirs | 由全局技能配置 `config/skills.json` 的 `extraDirs` 字段指定（参见 [config 模块](../config/README.md)） | 所有 Agent | 低 |
| Bundled | （非文件系统目录，编译时内嵌） | 所有 Agent（系统默认） | 最低 |

同名判定以目录名（即 skill 名）为准：同一目录名在不同优先级层级出现时，高优先级版本覆盖低优先级版本，低优先级版本被跳过并记录警告。

Agent 专属目录下的技能仅对该 Agent 可见，不影响其他 Agent。

### 磁盘加载

Session 启动时，磁盘加载层扫描前四层文件系统目录（ExtraDirs、Global、Agent 专属、Project）并按优先级从低到高加载。Bundled 技能不与文件系统目录一起扫描——编译时内嵌，通过 BuiltinSkillRegistry 独立加载。解析每个 SKILL.md 文件的 frontmatter，同名时高优先级覆盖低优先级。

正文采用按需加载策略：启动时只解析 frontmatter 并注册 skill 元数据，skill 被调用时才读取正文。

目录扫描仅遍历一级子目录下的 SKILL.md 文件，不递归子目录。扫描结果以 frontmatter 元数据缓存。扫描阶段的错误不导致 session 启动失败。

### 注册中心

采用双注册表架构：

- **DiskSkillRegistry**：管理磁盘加载的 skill，提供同步查询接口
- **BuiltinSkillRegistry**：管理编译期内置的 bundled skill

查询路由：先查 DiskSkillRegistry，未命中再查 BuiltinSkillRegistry。同名覆盖仅在同一注册表内生效；跨注册表的优先级由查询路由顺序保证——DiskSkillRegistry 优先于 BuiltinSkillRegistry。

技能文件变更在下一个 System Prompt 组装边界反映（新建会话、恢复会话、压缩重建，详见 [skill-listing-injection.md](skill-listing-injection.md)）；组装之间清单内容不变。

### 错误处理

磁盘加载阶段的错误不影响 session 正常运行。所有错误和警告信息输出到 stderr，格式包含来源路径和具体问题描述，供 User 定位问题原因：

- skill 目录路径不存在或无法访问 → 跳过该来源，记录提示
- 单个 SKILL.md 格式错误或必填字段缺失 → 跳过该 skill，其他 skill 正常加载
- 同名冲突 → 跳过低优先级版本，记录警告

## 数据流

### 启动加载

1. Session 启动，按优先级从低到高扫描四层文件系统目录：
   1. ExtraDirs 扫描（路径不存在 → 跳过）
   2. Global 目录扫描
   3. Agent 专属目录扫描
   4. Project 目录扫描
2. Bundled 技能通过 BuiltinSkillRegistry 独立加载（编译时内嵌，不走文件系统扫描）
3. 逐个解析每个 SKILL.md 的 frontmatter：
   - 格式错误或必填字段缺失 → 跳过该 skill 并记录
   - 同名覆盖 → 低优先级版本跳过并记录
4. 磁盘扫描的技能 → 写入 DiskSkillRegistry
5. 内置技能 → 写入 BuiltinSkillRegistry
6. 注册中心就绪，供 SP 组装和技能调用查询

### 按需加载正文

Skill 被 Agent 调用时，从注册中心查找 skill 实例，按需加载正文内容。磁盘技能从对应 SKILL.md 文件读取；内置技能从 BuiltinSkillRegistry 实例中直接获取。启动阶段不加载正文。

## 模块关系

- **上游**：Session 启动流程（触发磁盘加载）、Agent 配置（提供 agent-id）
- **下游**：文件系统（扫描目录、读取 SKILL.md 文件）、skill-listing-injection（消费注册中心数据生成技能清单，供 Agent 调度决策使用）、[skill-execution](skill-execution.md)（消费注册中心数据按需加载正文）
- **共享类型**：Skill 元数据结构（优先级层级、frontmatter 字段），定义于本文档，被 skill-listing-injection 和 skill-execution 消费
