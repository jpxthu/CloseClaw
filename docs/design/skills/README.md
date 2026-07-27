# Skills 模块

## 概述

关联需求文档：[../../requirements/skills.md](../../requirements/skills.md)

Skills 模块提供可复用技能插件体系——用户创建 SKILL.md 文件放入指定目录后，Agent 在下次 session 启动时自动发现并加载该技能。技能以纯 prompt 指令方式扩展 Agent 能力，不携带工具权限。

## 架构

Skills 模块由三个核心组件构成：磁盘加载层、注册中心层、执行层。技能按五层优先级来源组织——前四层为磁盘目录，Bundled 层为编译期内置数据。其中 ExtraDirs 为 Agent 配置中指定的外部复用路径。

```
五层技能来源（优先级从高到低）
  ├─ Project:  <project>/.closeclaw/skills/
  ├─ Agent:   ~/.closeclaw/agents/<id>/skills/
  ├─ Global:  ~/.openclaw/skills/
  ├─ ExtraDirs: 配置指定的外部目录
  └─ Bundled: 编译期内置（不走磁盘加载）
  ↓
磁盘加载层（Disk Loader）
  → 扫描四层磁盘目录 → 解析 SKILL.md frontmatter → 同名高优先级覆盖低优先级
  ↓
注册中心层（Skill Registry）
  → 双注册表：DiskSkillRegistry（磁盘） + BuiltinSkillRegistry（内置，独立加载 Bundled 数据）
  → 提供统一查询路由
  ↓
执行层（Execution）
  → Agent 调用 SkillTool → 加载正文 → inline 执行 → 结果返回
```

热重载由文件监听器触发，检测到 SKILL.md 变更后使缓存失效并增量更新注册中心，具体流程见 [skill-listing-injection.md](skill-listing-injection.md)。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [skill-definition.md](skill-definition.md) | Skill 定义：frontmatter 字段、目录优先级、磁盘加载、注册中心、错误处理 |
| [skill-listing-injection.md](skill-listing-injection.md) | 技能清单生成：过滤、排序、格式化、文件监听触发热重载 |
| [skill-execution.md](skill-execution.md) | 技能调用：inline 执行流程、正文按需加载、SkillCreator 工具 |

## 数据流

### 加载与注册

Session 启动时，磁盘加载层按优先级从低到高依次扫描：先加载低优先级层（内置），后加载高优先级层（项目级）。实际扫描顺序为 Bundled → ExtraDirs → Global → Agent → Project。BuiltinSkillRegistry 在启动时由编译期嵌入的内置数据初始化，不参与磁盘扫描。高优先级层中的同名技能覆盖低优先级层中已加载的。加载完成后注册中心冻结，技能集合在 session 内不可变（热重载除外）。

### 技能调用

Agent 决策调用某个技能 → 通过 SkillTool 发起调用 → 从注册中心查找技能实例 → 按需加载技能正文 → 正文注入对话上下文 → Agent 按指令继续执行。

技能正文统一采用 inline 执行——直接展开到当前 Agent 上下文，不创建隔离子 Agent。

## 模块关系

- **上游**：Agent 运行时（调度技能调用）、Session 模块（查询技能清单以生成 per-turn attachment）
- **下游**：文件系统（扫描目录、读取 SKILL.md）
- **无关**：processor_chain（skill 不参与消息出站处理）、renderer（skill 不参与平台渲染）、system_prompt（技能清单不进入 system prompt 任何分区）、权限引擎（Agent 运行时校验工具权限，skills 模块不直接调用）
- **共享类型**：无
