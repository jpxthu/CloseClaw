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
| [skill-definition.md](skill-definition.md) | Skill 定义：frontmatter 配置、执行模式、目录层级与优先级 |

## 架构

Skills 模块由四个核心组件构成：磁盘加载层、注册中心层、注入层、执行层。

```
磁盘文件系统
  │
  ├─ ~/.closeclaw/skills/           ← 全局 skill
  ├─ ~/.closeclaw/agents/<id>/skills/ ← agent 专属 skill
  ├─ <project>/.closeclaw/skills/   ← 项目级 skill
  ├─ ExtraDirs 配置路径              ← 外部复用 skill
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

## 数据流

### 加载与注入

Session 启动时，磁盘加载层扫描五层目录、解析 SKILL.md 并写入注册中心。随后注入层从注册中心获取 skill 集合，生成 listing 作为 SkillListingSection 注入 system prompt 静态层。完整注入流程见 [skill-listing-injection.md](skill-listing-injection.md)。

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

热重载跨模块协作，分两阶段：

1. **skills 模块**：文件变更事件触发 → 300ms debounce → 使注册中心 listing 缓存失效 → 重新扫描变更目录 → 重新生成 listing。不下发信号、不通知 SessionManager。
2. **system prompt 构建器**：下次 session 创建、archive 恢复或 compaction 发生时，重新调用 build_from_workspace，从 registry 获取最新 listing，生成新的 SkillListingSection 替换 system prompt 中的旧版本。

不涉及 session transcript 操作，无需查找或修改 transcript 中的消息。

## 模块关系

- **上游**：agent 运行时（调度 skill 调用）、system prompt 构建器（注入 skill listing）、session 管理器（启动时加载、关闭时释放）
- **下游**：文件系统（扫描目录、读取 SKILL.md）、sub-agent 管理（fork 模式下创建隔离子 agent）
- **相关**：权限引擎（agent 运行时在执行 skill 前向权限引擎校验 allowed-tools，skills 模块不直接调用权限引擎）
- **无关**：processor_chain（skill 不参与消息出站处理）、renderer（skill 不参与平台渲染）
