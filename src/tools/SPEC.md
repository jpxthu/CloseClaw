# tools — 两级工具体系

## 模块概述

`tools` 模块为 CloseClaw 实现两级工具体系：LLM 可调用的能力（Tool）与领域知识按需读取（Skill）完全独立。

核心设计：
- `Tool` trait 定义工具的核心接口，所有工具必须实现 `Send + Sync + 'static`
- `ToolRegistry` 是并发安全的注册中心，内部用 `tokio::sync::RwLock` 包裹 `HashMap<String, Arc<dyn Tool>>`
- `ToolDescriptor` 仅含 name / group / summary / is_deferred，用于 system prompt 一级索引
- 工具 detail 和 input_schema 按需通过 `ToolSearch` 触发注入，不在一级索引展开
- `builtin` 子模块提供 5 个 file_ops 工具和 2 个 meta 工具，全部通过 `register_builtin_tools()` 一键注册
- System prompt 集成：builder.rs 提供 `build_tools_section(registry, ctx)` async 函数，返回 `Section::ToolsSection`；`build_from_workspace` 中预埋 `Section::ToolsSection(String::new())` 占位符（位于 RoleSection 之后 MemorySection 之前）。当前 `build_tools_section` 尚未在 sync 路径中与 `build_from_workspace` 打通，内容通过 dynamic_sections 外部传入

边界：builtin tools 不依赖 `crate::skills` 模块；`ToolRegistry` 依赖 `tokio`（异步运行时）。

---

## 公开接口

### 核心类型（mod.rs）

- `Tool` trait — 工具核心接口，6 个方法：name / group / summary / detail / input_schema / flags
- `ToolFlags` — bitflags 风格运行时标记（is_concurrency_safe / is_read_only / is_destructive / is_expensive / is_deferred_by_default）
- `ToolContext` — 运行时上下文（agent_id + workdir）
- `ToolDescriptor` — 一级摘要数据（name / group / summary / is_deferred）
- `ToolError` — 工具层错误类型，用 thiserror 定义（NotFound / AlreadyRegistered / Serialization / Io）

### 注册中心（registry.rs）

- `ToolRegistry::new()` — 创建空注册表
- `ToolRegistry::register(tool)` — 注册工具，冲突返回 `AlreadyRegistered`
- `ToolRegistry::list_descriptors(ctx)` — 列出所有 ToolDescriptor，按 ctx 过滤
- `ToolRegistry::get_detail(name)` — 获取指定工具的 detail 字符串，不存在返回 `NotFound`
- `ToolRegistry::list_by_group(group)` — 列出指定分组下的所有工具名
- `ToolRegistry::build_tools_section(ctx)` — 生成分组索引字符串，超 1500 字符截断

### 内建工具（builtin/）

- `register_builtin_tools(registry)` — 将全部 7 个内建工具注册到指定注册表
- `ReadTool` / `WriteTool` / `EditTool` / `GrepTool` / `LsTool` — file_ops 组，group = "file_ops"
- `ToolSearchTool` / `PermissionQueryTool` — meta 组，group = "meta"，is_deferred_by_default = false

---

## 架构与结构

### 子模块划分

```
tools/
├── mod.rs          # Tool trait、ToolFlags、ToolContext、ToolDescriptor、ToolError
├── registry.rs     # ToolRegistry 并发注册中心 + build_tools_section
└── builtin/
    ├── mod.rs      # register_builtin_tools 统一入口
    ├── file_ops.rs # Read / Write / Edit / Grep / Ls
    ├── search.rs   # ToolSearchTool
    └── permission.rs # PermissionQueryTool
```

### 两级设计

**一级索引**（`ToolRegistry::build_tools_section` 输出）：按 group 聚合，展示 `**{group}** — (always loaded)` 标题 + 工具名列表（中文顿号分隔、按名称排序），供 LLM 了解可用工具范围。总长不超过 1500 字符，超长截断并附加 `... (N more tools, use ToolSearch to explore)` 提示。

**二级详情**（`get_detail` 返回）：完整 detail 描述 + input_schema JSON，通过 `ToolSearch` 按关键词或精确名触发注入。

### 关键数据不变式

- `ToolFlags::is_eager()` 返回 `!is_deferred_by_default`
- `build_tools_section` 按 group 名排序，保证输出稳定
- `register_builtin_tools` 中 file_ops 工具不依赖 `crate::skills`
