# tools — 两级工具体系

## 模块概述

`tools` 模块为 CloseClaw 实现两级工具体系：LLM 可调用的能力（Tool）与领域知识按需读取（Skill）完全独立。

核心设计：
- `Tool` trait 定义工具的核心接口，所有工具必须实现 `Send + Sync + 'static`
- `ToolRegistry` 是并发安全的注册中心，内部用 `tokio::sync::RwLock` 包裹 `HashMap<String, Arc<dyn Tool>>`
- `ToolDescriptor` 仅含 name / group / summary / is_deferred，用于 system prompt 一级索引
- 工具 detail 和 input_schema 按需通过 `ToolSearch` 触发注入，不在一级索引展开
- `builtin` 子模块提供 5 个 file_ops 工具、2 个 meta 工具、5 个 git_ops 工具、1 个 SkillTool 和 2 个 stub 工具，全部通过 `register_builtin_tools()` 一键注册
- System prompt 集成：builder.rs 提供 `build_tools_section(registry, ctx)` async 函数，返回 `Section::ToolsSection`；`build_from_workspace` 中预埋 `Section::ToolsSection(String::new())` 占位符（位于 RoleSection 之后 MemorySection 之前）。当前 `build_tools_section` 尚未在 sync 路径中与 `build_from_workspace` 打通，内容通过 dynamic_sections 外部传入

边界：builtin tools 不依赖 `crate::skills` 模块；`ToolRegistry` 依赖 `tokio`（异步运行时）。

---

## 公开接口

### 核心类型（mod.rs）

- `Tool` trait — 工具核心接口，6 个方法：name / group / summary / detail / input_schema / flags；第 7 个方法 call() 用于执行（默认返回 NotImplemented）
- `ToolFlags` — bitflags 风格运行时标记（is_concurrency_safe / is_read_only / is_destructive / is_expensive / is_deferred_by_default）
- `ToolContext` — 运行时上下文（agent_id + workdir）
- `ToolDescriptor` — 一级摘要数据（name / group / summary / is_deferred）
- `ToolError` — 工具层错误类型，用 thiserror 定义（NotFound / AlreadyRegistered / Serialization / Io）
- `ToolCallError` — 工具执行错误（NotFound / PermissionDenied / InvalidArgs / ExecutionFailed / NotImplemented）
- `ToolMessage` — 注入上下文的元消息（content + is_meta）
- `ContextModifier` — 上下文修改器（allowed_tools 列表）
- `ToolResult` — 工具执行结果（data + new_messages + context_modifier）

### 注册中心（registry.rs）

- `ToolRegistry::new()` — 创建空注册表
- `ToolRegistry::register(tool)` — 注册工具，冲突返回 `AlreadyRegistered`
- `ToolRegistry::list_descriptors(ctx)` — 列出所有 ToolDescriptor，按 ctx 过滤
- `ToolRegistry::get_detail(name)` — 获取指定工具的 detail 字符串，不存在返回 `NotFound`
- `ToolRegistry::list_by_group(group)` — 列出指定分组下的所有工具名
- `ToolRegistry::build_tools_section(ctx)` — 生成分组索引字符串，超 1500 字符截断

### 内建工具（builtin/）

- `SkillTool` — skills 组，从 DiskSkillRegistry 查找 skill 并注入 SKILL.md 内容到 agent 上下文；group = "skills"，is_deferred_by_default = false；输入参数 skill_name（必填）和 args（可选）
- `register_builtin_tools(registry, disk_registry)` — 将全部 16 个内建工具注册到指定注册表
- `ReadTool` / `WriteTool` / `EditTool` / `GrepTool` / `LsTool` — file_ops 组，group = "file_ops"
- `ToolSearchTool` / `PermissionQueryTool` — meta 组，group = "meta"，is_deferred_by_default = false
- `GitStatusTool` / `GitLogTool` / `GitCommitTool` / `GitPushTool` / `GitPullTool` — git_ops 组，group = "git_ops"；GitStatusTool/GitLogTool 标记 is_read_only，GitCommitTool/GitPushTool/GitPullTool 标记 is_destructive
- `CodingAgentTool` — coding_agent 组，is_deferred_by_default
- `SkillCreatorTool` — skill_creator 组，is_deferred_by_default + is_destructive

---

## 架构与结构

### 子模块划分

```
tools/
├── mod.rs              # Tool trait、ToolFlags、ToolContext、ToolDescriptor、ToolError
├── registry.rs         # ToolRegistry 并发注册中心 + build_tools_section
└── builtin/
    ├── mod.rs          # register_builtin_tools 统一入口（15 个工具）
    ├── file_ops.rs     # Read / Write / Edit / Grep / Ls
    ├── git_ops.rs      # GitStatus / GitLog / GitCommit / GitPush / GitPull
    ├── coding_agent.rs # CodingAgentTool (stub)
    ├── skill_creator.rs # SkillCreatorTool (stub)
    ├── skill_tool.rs   # SkillTool
    ├── search.rs       # ToolSearchTool
    └── permission.rs   # PermissionQueryTool
```

### 两级设计

**一级索引**（`ToolRegistry::build_tools_section` 输出）：按 group 聚合，展示 `**{group}** — (always loaded)` 标题 + 工具名列表（中文顿号分隔、按名称排序），供 LLM 了解可用工具范围。总长不超过 1500 字符，超长截断并附加 `... (N more tools, use ToolSearch to explore)` 提示。

**二级详情**（`get_detail` 返回）：完整 detail 描述 + input_schema JSON，通过 `ToolSearch` 按关键词或精确名触发注入。

### 关键数据不变式

- `ToolFlags::is_eager()` 返回 `!is_deferred_by_default`
- `build_tools_section` 按 group 名排序，保证输出稳定
- `register_builtin_tools` 中 builtin 工具不依赖 `crate::skills`
