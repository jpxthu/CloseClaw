# Skills 模块规格书

> 本文件描述 `src/skills/` 模块的当前实现状态，不代表设计意图或未来计划。

## 1. 模块概述

Skills 模块为 Agent 提供可复用工具能力，采用插件化架构。所有技能实现统一的 `Skill` trait，通过 `SkillRegistry` 注册中心统一管理。

核心类型：
- `Skill` trait — 所有技能必须实现的接口
- `SkillRegistry` — 技能注册表，支持并发访问
- `SkillManifest` — 技能元数据（名称、版本、描述）

子模块：
- `registry` — Skill trait + SkillRegistry 注册中心
- `builtin` — 7 个内置技能实现（file_ops, git_ops, search, permission, discovery）
- `coding_agent` — 编码委托技能（stub）
- `skill_creator` — 技能创建辅助技能

---

## 2. 公开接口

### 数据类型

| 类型 | 所属 | 功能 |
|------|------|------|
| `Skill` | registry | 技能 trait，定义 `manifest()` / `methods()` / `execute()` 接口 |
| `SkillManifest` | registry | 技能元数据（name、version、description、author、dependencies） |
| `SkillInput` | registry | 技能执行输入（skill_name、method、args、agent_id） |
| `SkillOutput` | registry | 技能执行输出（success、result、error） |
| `SkillError` | registry | 错误类型：`NotFound`、`MethodNotFound`、`ExecutionFailed`、`InvalidArgs`、`PermissionDenied` |

### 构造

| 接口 | 所属 | 功能 |
|------|------|------|
| `SkillRegistry::new()` | SkillRegistry | 创建空注册表 |
| `SkillRegistry::default()` | SkillRegistry | 创建默认注册表（同 `new()`） |
| `FileOpsSkill::new()` | FileOpsSkill | 创建无权限引擎实例 |
| `FileOpsSkill::with_engine(engine)` | FileOpsSkill | 创建带权限引擎实例 |
| `GitOpsSkill::new()` | GitOpsSkill | 创建实例 |
| `SearchSkill::new()` | SearchSkill | 创建 stub 实例 |
| `PermissionSkill::new()` | PermissionSkill | 创建无权限引擎实例 |
| `PermissionSkill::with_engine(engine)` | PermissionSkill | 创建带权限引擎实例 |
| `SkillDiscoverySkill::new()` | SkillDiscoverySkill | 创建无权限引擎实例 |
| `SkillDiscoverySkill::with_engine(engine)` | SkillDiscoverySkill | 创建带权限引擎实例 |
| `CodingAgentSkill::new(model: Option<String>)` | CodingAgentSkill | 创建实例，`None` 时使用默认模型 minimax/MiniMax-M2.7 |
| `SkillCreatorSkill::new()` | SkillCreatorSkill | 创建实例 |
| `builtin_skills()` | builtin | 获取全部 7 个内置技能（无引擎） |
| `builtin_skills_with_engine(engine)` | builtin | 获取全部 7 个内置技能（带引擎） |

### 内置类型

| 类型 | 所属 | 功能 |
|------|------|------|
| `FileOpsSkill` | builtin | 文件系统操作技能 |
| `GitOpsSkill` | builtin | Git 操作技能 |
| `SearchSkill` | builtin | 搜索技能（stub） |
| `PermissionSkill` | builtin | 权限查询技能 |
| `SkillDiscoverySkill` | builtin | ClawHub 技能发现技能 |
| `BuiltinSkills` | builtin | 内置技能聚合类型，提供 `all()` / `all_with_engine()` 工厂方法 |

### 主操作

| 接口 | 所属 | 功能 |
|------|------|------|
| `SkillRegistry::register(skill)` | SkillRegistry | 注册技能 |
| `SkillRegistry::unregister(name)` | SkillRegistry | 注销技能 |
| `Skill::execute(method, args)` | Skill trait | 执行技能方法 |
| `file_ops: read` | FileOpsSkill | 读取文件 |
| `file_ops: write` | FileOpsSkill | 写入文件 |
| `file_ops: delete` | FileOpsSkill | 删除文件 |
| `file_ops: exists` | FileOpsSkill | 检查文件存在性 |
| `file_ops: list` | FileOpsSkill | 列出目录内容 |
| `git_ops: status` | GitOpsSkill | git status --porcelain |
| `git_ops: log` | GitOpsSkill | git log --oneline -10 |
| `git_ops: commit` | GitOpsSkill | git commit |
| `git_ops: push` | GitOpsSkill | git push |
| `git_ops: pull` | GitOpsSkill | git pull |
| `search: search` | SearchSkill | 搜索（stub，永远返回空） |
| `permission_query: query` | PermissionSkill | 查询指定 agent/action 的权限 |
| `permission_query: list_actions` | PermissionSkill | 列出所有支持的动作 |
| `skill_discovery: find` | SkillDiscoverySkill | 从 ClawHub 搜索技能 |
| `skill_discovery: install` | SkillDiscoverySkill | 安装技能（需 spawn 权限） |
| `skill_discovery: list` | SkillDiscoverySkill | 列出已安装技能 |
| `skill_discovery: update` | SkillDiscoverySkill | 更新技能 |
| `coding_agent: delegate` | CodingAgentSkill | 委托编码任务（stub） |
| `coding_agent: review` | CodingAgentSkill | 代码审查（stub） |
| `coding_agent: refactor` | CodingAgentSkill | 代码重构（stub） |
| `coding_agent: test` | CodingAgentSkill | 生成测试（stub） |
| `skill_creator: guide` | SkillCreatorSkill | 返回创建技能指南 |
| `skill_creator: template` | SkillCreatorSkill | 返回 SKILL.md 模板 |
| `skill_creator: validate` | SkillCreatorSkill | 验证代码结构 |

### 查询

| 接口 | 所属 | 功能 |
|------|------|------|
| `SkillRegistry::get(name)` | SkillRegistry | 按名称查找技能 |
| `SkillRegistry::list()` | SkillRegistry | 列出所有已注册技能名 |
| `SkillRegistry::contains(name)` | SkillRegistry | 检查技能是否存在 |
| `Skill::manifest()` | Skill trait | 获取技能元数据 |
| `Skill::methods()` | Skill trait | 列出技能支持的方法 |

---

## 3. 架构与结构

### 子模块划分

- **`registry`**：核心类型定义（Skill trait、SkillRegistry、SkillManifest、SkillInput、SkillOutput、SkillError）
- **`builtin`**：7 个内置技能实现 + BuiltinSkills 聚合
- **`coding_agent`**：CodingAgentSkill（stub 状态）
- **`skill_creator`**：SkillCreatorSkill

### 内置技能

| 技能 | 功能 | 权限控制 |
|------|------|----------|
| `file_ops` | 文件系统读写删列表存在性检查 | 可选（注入 PermissionEngine） |
| `git_ops` | git status/log/commit/push/pull | 无 |
| `search` | Web 搜索 | — |
| `permission_query` | Agent 自身权限查询 | 可选 |
| `skill_discovery` | ClawHub 技能市场 | `install` 检查 `spawn` 权限 |
| `coding_agent` | 编码任务委托 | — |
| `skill_creator` | 技能创建指导 | — |

### 数据流

Skill 执行入口：`Skill::execute(method, args)` → 内部路由到具体方法 → 返回 `Result<serde_json::Value, SkillError>`

`SkillRegistry` 内部使用 `tokio::sync::RwLock` 保护 `HashMap<String, Arc<dyn Skill>>`，所有操作均为 async。

---

## 4. 已知限制

- `search` 技能为 stub，不执行真实搜索
- `coding_agent` 技能为 stub，不真实调用编码 agent
- `git_ops` 无权限控制
- `builtin_skills()` 不接受 `agent_id`，无法针对特定 Agent 过滤技能
- `skill_discovery` 依赖系统已安装 `clawhub` CLI
