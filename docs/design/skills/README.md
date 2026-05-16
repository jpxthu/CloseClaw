# Skills 模块

## 概述

Skills 模块是 CloseClaw 的 agent 可复用能力插件体系。用户创建 SKILL.md 文件放入指定目录即可自动被发现和加载，无需修改代码即可扩展 agent 能力。

核心设计：
- 磁盘即插即用，放 SKILL.md 到对应目录即生效
- 五层优先级目录结构，同名冲突按优先级取高
- frontmatter 配置驱动行为（描述、权限、执行模式、触发条件等）
- skills 和 tools 统一注册中心，共享调用协议
- 双执行模式：inline（当前 agent 上下文）和 fork（隔离子 agent）
- 多 agent 独立，各自有独立的 skill 空间

### 子功能索引

| 文档 | 内容 |
|------|------|
| [skill-listing-injection.md](skill-listing-injection.md) | Skill 列表注入：session 启动时的列表注入、热重载替换、compaction 保护、条件激活 |

## 架构

Skills 模块由五个核心组件构成：Skill 定义层、磁盘加载层、注册中心层、执行层、注入层。

```
磁盘文件系统
  │
  ├─ ~/.closeclaw/skills/           ← 全局 skill
  ├─ ~/.closeclaw/agents/<id>/skills/ ← agent 专属 skill
  ├─ <project>/.closeclaw/skills/   ← 项目级 skill
  ├─ extraDirs 配置路径              ← 外部复用 skill
  └─ bundled/                        ← 编译期内置 skill
        │
        ▼
磁盘加载层（Disk Loader）
  → 扫描五层目录 → 解析 SKILL.md frontmatter → 同名冲突去重
        │
        ▼
注册中心层（Skill Registry）
  → 统一管理 disk skill 与 bundled skill → 提供查询路由
        │
        ▼
注入层（Injection）
  → session 启动时注入 skill listing → 让 agent 获知可用 skill
        │
        ▼
执行层（Execution）
  → agent 决策调用 SkillTool → inline 或 fork 执行 → 结果返回
```

### Skill 定义

每个 skill 由三部分组成：配置清单（frontmatter）、正文（SKILL.md 中 frontmatter 之后的指令文本）、执行入口。

下文中"正文"均指 SKILL.md 的指令文本部分，区别于 frontmatter 配置。

**配置清单**（frontmatter）包含以下字段：

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
| user-invocable | 否 | 是否可通过 slash command 调用 |

**执行模式**两种：
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

### 磁盘加载

Session 启动时按优先级从低到高扫描全部五个来源目录，每层覆盖上一层的同名 skill。加载完成后注册中心冻结，skill 集合在 session 内不可变（热重载除外）。

优化方案：可采用两阶段加载——启动时只注册 skill listing（名称、描述、来源、条件路径），调用时再按需加载正文。

### 注册中心

采用双注册表架构过渡：

- **SkillRegistry**（已有）：管理 bundled skill，异步操作
- **DiskSkillRegistry**（新增）：管理磁盘加载的 skill，同步查询

查询路由：先查 DiskSkillRegistry，未命中再查 SkillRegistry。

### 权限控制

权限检查不在 skills 模块内实现，而是由上游 agent 运行时调用权限引擎校验。Skill 的 frontmatter 声明 allowed-tools 字段，权限引擎据此限制 skill 可用工具集。

### 错误处理

磁盘加载阶段的错误不影响 session 正常运行：

- extraDirs 路径不存在：记录警告，跳过该来源
- 单个 SKILL.md 格式错误：记录错误，跳过该 skill，其他 skill 正常加载
- 同名冲突：跳过低优先级版本，记录警告
- 必填字段缺失（description）：视为严重错误，启动时报告但不阻止 session 继续

## 数据流

### 加载与注入

```
Session 启动
  → 扫描五层目录（bundled → extraDirs → global → agent → project）
    → 每层解析 SKILL.md frontmatter
      → 同名冲突去重（高优先级覆盖）
        → 写入注册中心
          → 生成 skill listing 字符串（名称 + 描述 + 触发条件）
            → 作为 system-reminder 消息注入 session 最开头
              → agent 在对话初始化时看到全部可用 skill
```

### 技能调用

```
Agent 决策调用某个 skill
  → 通过 SkillTool 发起调用
    → 从注册中心查找 skill 实例
      → 权限校验（allowed-tools）
        → 按需加载 skill 正文（如采用两阶段加载）
          → 判断执行模式
            ├─ inline：正文内容展开到 agent 上下文，权限修改留在当前 session
            └─ fork：创建隔离子 agent，注入 allowed-tools 权限，在子 agent 中执行
              → 执行结果返回
                → 注入到对话上下文供 agent 继续处理
```

### 热重载

```
文件系统变更（SKILL.md 新增/修改/删除）
  → 文件监听器检测到变更（300ms debounce）
    → 使 skill listing 缓存失效
      → 重新生成 skill listing
        → 替换 session 开头的旧 listing 消息
          → agent 在下一轮对话中看到更新后的 skill 列表
```

## 模块关系

- **上游**：agent 运行时（调度 skill 调用）、system prompt 构建器（注入 skill listing）、session 管理器（启动时加载、关闭时释放）
- **下游**：文件系统（扫描目录、读取 SKILL.md）、sub-agent 管理（fork 模式下创建隔离子 agent）、权限引擎（执行前校验 allowed-tools）
- **无关**：processor_chain（skill 不参与消息出站处理）、renderer（skill 不参与平台渲染）
