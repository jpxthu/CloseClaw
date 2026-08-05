# Skills 模块

## 概述

关联需求文档：[../../requirements/skills.md](../../requirements/skills.md)

Skills 模块提供可复用技能插件体系——用户创建 SKILL.md 文件放入指定目录后，Agent 在下次 session 启动时自动发现并加载该技能。用户创建的磁盘技能以纯 prompt 指令方式扩展 Agent 能力，不携带工具权限。

## 架构

Skills 模块由三个核心组件构成：磁盘加载层、注册中心层、执行层。技能按五层优先级来源组织——前四层为磁盘目录，Bundled 层为编译期内置数据。其中 ExtraDirs 为由配置指定的外部复用路径。

```
五层技能来源（优先级从高到低）
  ├─ Project:  <project>/.closeclaw/skills/
  ├─ Agent:   ~/.closeclaw/agents/<id>/skills/
  ├─ Global:  ~/.closeclaw/skills/
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

### 子功能索引

| 文档 | 内容 |
|------|------|
| [skill-definition.md](skill-definition.md) | Skill 定义：frontmatter 字段、目录优先级、磁盘加载、注册中心、错误处理 |
| [skill-listing-injection.md](skill-listing-injection.md) | 技能清单生成：过滤、排序、格式化 |
| [skill-execution.md](skill-execution.md) | 技能调用：inline 执行、Bundled 原生执行、SkillCreator 技能、斜杠命令调用 |

## 数据流

### 加载与注册

Session 启动时，磁盘加载层按优先级从低到高依次扫描四层文件系统目录。实际扫描顺序为 ExtraDirs → Global → Agent → Project（优先级从低到高）。BuiltinSkillRegistry 在启动时由编译期嵌入的内置数据初始化，Bundled 技能不参与磁盘扫描，通过 BuiltinSkillRegistry 独立加载。高优先级层中的同名技能覆盖低优先级层中已加载的。加载完成后注册中心冻结，技能集合在 session 内不可变。

### 技能清单生成

SP Builder 触发清单生成 → 列表生成器从注册中心获取 skill 元数据 → 过滤（user-invocable + 已激活条件技能）→ 排序（来源优先级降序 → 字母序）→ 格式化 → 返回给 SP Builder 注入 SkillsSection。详见 [skill-listing-injection.md](skill-listing-injection.md)。

### 技能调用

Agent 决策调用某个技能 → 通过 SkillTool 发起调用 → 从注册中心查找技能实例 → 按技能类型分流：

- 磁盘技能：按需加载正文 → 变量替换 → 正文注入对话上下文 → Agent 按指令继续执行
- Bundled 技能：trait 方法分发执行 → 结果以 meta message 注入对话上下文

详见 [skill-execution.md](skill-execution.md)。

## 模块关系

- **上游**：Agent 运行时（Agent-in-Session 决策调用技能）、Agent 配置（提供 agent-id）、文件系统（提供 SKILL.md 数据源）
- **下游**：System Prompt 模块（消费技能清单渲染结果，注入 SkillsSection）
- **无关**：processor_chain（skill 不参与消息出站处理）、renderer（skill 不参与平台渲染）、权限引擎（Agent 运行时校验工具权限和管理技能白名单；skills 模块作为纯数据提供方，不反向查询权限状态）
- **共享类型**：Skill 元数据结构（优先级层级、frontmatter 字段），定义于 [skill-definition.md](skill-definition.md)
