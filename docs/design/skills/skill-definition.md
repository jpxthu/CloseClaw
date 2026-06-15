# Skill 定义

## 概述

Skill 的配置清单（frontmatter）、正文结构、执行模式、目录层级与优先级。

## 架构

每个 skill 由三部分组成：配置清单（frontmatter）、正文（SKILL.md 中 frontmatter 之后的指令文本）、执行入口。

### 配置清单

| 字段 | 必填 | 作用 |
|------|------|------|
| description | 是 | 供 agent 决策是否调用 |
| name | 否 | 默认取目录名 |
| allowed-tools | 否 | 限制 skill 可用的工具，不填则无限制 |
| when-to-use | 否 | 决策提示，帮助 agent 判断调用时机 |
| context | 否 | 执行模式：inline（默认）或 fork |
| agent | 否 | fork 模式下使用的 agent 类型 |
| agent-id | 否 | 限定只有特定 agent 可用 |
| effort | 否 | 成本估算，供 agent 调度参考 |
| paths | 否 | 条件激活的文件 glob 匹配模式 |
| user-invocable | 否 | 是否可通过 slash command 调用。默认 false（不声明则不出现在 listing 中，但 agent 仍可通过 whenToUse/paths 条件自动触发调用） |

### 执行模式

- **inline**：在当前 agent 上下文中执行，skill 内容展开到 agent prompt，权限修改留在当前 session
- **fork**：在独立 sub-agent 中执行，上下文隔离，allowed-tools 不泄露到父 agent

### 目录层级与优先级

五个技能来源按优先级从高到低排列：

| 层级 | 路径 | 作用域 | 优先级 |
|------|------|--------|--------|
| Project | `<project-root>/.closeclaw/skills/` | 仅该项目 | 最高 |
| Agent | `~/.closeclaw/agents/<agent-id>/skills/` | 仅该 agent | 高 |
| Global | `~/.closeclaw/skills/` | 所有 agent | 中 |
| ExtraDirs | 配置路径（如复用外部工具链的 skill 目录） | 由配置决定 | 低 |
| Bundled | 编译期内置 | 全局内建 | 最低 |

同名冲突处理：高优先级生效，低优先级被跳过并记录警告。

### 注册中心

采用双注册表架构过渡：

- **SkillRegistry**（已有）：管理 bundled skill，异步操作
- **DiskSkillRegistry**（新增）：管理磁盘加载的 skill，同步查询

查询路由：先查 DiskSkillRegistry，未命中再查 SkillRegistry。

### 权限控制

权限检查不在 skills 模块内实现，而是由 agent 运行时在执行 skill 前调用权限引擎校验。skills 模块通过 frontmatter 的 allowed-tools 字段声明 skill 可使用的工具范围，供权限引擎在 skill 执行时参照校验。skills 模块不直接调用权限模块。

### 错误处理

磁盘加载阶段的错误不影响 session 正常运行：

- extraDirs 路径不存在：记录警告，跳过该来源
- 单个 SKILL.md 格式错误：记录错误，跳过该 skill，其他 skill 正常加载
- 同名冲突：跳过低优先级版本，记录警告
- 必填字段缺失（description）：视为严重错误，启动时报告但不阻止 session 继续

### 对外工具

Skills 模块暴露 `register_tools(registry)` 方法，由 tools 模块在启动编排时调用，向 ToolRegistry 注册以下工具：

| 工具 | 分组 | 说明 | 加载策略 |
|------|------|------|---------|
| SkillTool | skills | agent 通过此工具调用 skill，返回 skill 正文和指令 | 始终加载 |
| SkillCreator | skill_creator | agent 通过此工具创建或修改 skill 文件 | 延迟加载 |

## 数据流

### 磁盘加载

Session 启动时，按五层优先级从低到高依次扫描：先加载低优先级层，后加载高优先级层。高优先级层中的同名 skill 覆盖低优先级层中已加载的，最终生效的始终是最高优先级的版本。加载完成后注册中心冻结，skill 集合在 session 内不可变（热重载除外）。

### 优化方案

可采用两阶段加载——启动时只注册 skill listing（名称、描述、来源、条件路径），调用时再按需加载正文。

## 模块关系

- **与 skill-listing-injection**：skill 定义是注入层的数据来源，注入层从注册中心读取 skill 定义的 name、description、条件路径等字段生成 listing
- **与执行层**：frontmatter 的 context 字段决定执行模式（inline/fork），allowed-tools 字段供执行层在调用前进行权限校验
- **与磁盘加载层**：目录层级定义决定加载优先级，加载层按五层路径从低到高扫描并去重
- **跨模块：权限引擎**：skills 模块通过 allowed-tools 声明工具范围，由 agent 运行时在执行前调用权限引擎校验
- **跨模块：tools 模块**：tools 模块在启动编排时调用 register_tools 注册 SkillTool 和 SkillCreator
