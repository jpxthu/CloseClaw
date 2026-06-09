# Tools 模块

## 概述

Tools 模块是 CloseClaw 的工具注册基础设施——提供统一接口、并发安全的注册中心、索引构建和工具发现能力。各模块通过 ToolRegistry 自行注册工具，ToolRegistry 不硬编码任何工具列表。

核心设计：
- 所有工具通过统一的 Tool trait 实现
- ToolRegistry 是全局注册中心，各模块在初始化时调用 `register()` 注册自身工具
- 一级索引注入 system prompt，展示工具分组、名称和简要描述
- 二级详情通过工具发现机制按需注入，不占用初始上下文

### 子功能索引

| 文档 | 内容 |
|------|------|
| [bash-tool.md](bash-tool.md) | Bash 工具：命令执行、超时控制、输出截断、后台触发 |
| [bash-security.md](bash-security.md) | Bash 安全解析：AST 分析、信任分级、攻击检测 |
| [background-tasks.md](background-tasks.md) | 后台任务：异步执行、自动后台化、卡死检测、完成通知 |
| [tools-prompt-injection.md](tools-prompt-injection.md) | 工具提示词注入：两级注入机制、加载策略、长度控制 |
| [dynamic-prompt-generation.md](dynamic-prompt-generation.md) | 提示词动态生成：Schema/Prompt 双轨制、上下文感知 |
| [tools-keywords.md](tools-keywords.md) | 工具关键词索引：嵌入格式、匹配机制、维护原则 |

## 架构

Tools 模块由三层组成：接口层、注册中心层、工具提供者层。

```
Tool trait（接口层）          ← 定义工具的统一接口
    │
ToolRegistry（注册中心层）    ← 并发安全的注册、查询、索引构建
    │
工具提供者层                  ← 各模块自行注册的工具
```

### 接口层

每个工具都实现统一的 Tool trait，包含一组描述方法和一个执行方法：

- **标识**（`name` / `group`）：工具名和所属分组，用于索引和发现
- **摘要**（`summary`）：一句话描述，用于工具列表场景
- **行为描述**（`detail`）：完整的功能说明，常用工具的行为描述进入一级索引供 LLM 理解工具用途
- **动态 prompt 生成**（`generate_prompt`）：根据运行时上下文（权限、可用工具、工作目录等）动态调整工具描述，默认实现回退到 `detail`
- **参数模式**（`input_schema`）：JSON Schema 格式，直接暴露为 API schema，不转自然语言（动态生成机制详见 [dynamic-prompt-generation.md](dynamic-prompt-generation.md)）
- **运行时标记**（`flags`）：标识工具是否只读、是否破坏性、是否默认延迟加载、是否并发安全

### 注册中心层

ToolRegistry 是线程安全的全局工具注册与查询入口，内部以工具名为键管理所有已注册工具。提供以下能力：

- **注册**：启动时各模块一键注册自身工具，冲突时报错
- **索引构建**：按分组聚合工具，生成一级索引字符串。常用工具展示名称和行为描述，延迟加载工具仅展示名称和危险度标记。按分组排序，组内按名称排序，保证每次输出稳定
- **详情查询**：按工具名获取完整详情，供工具发现机制触发注入
- **分组查询**：按分组名获取该组下所有工具名

各模块在启动时自行调用 ToolRegistry 注册工具。ToolRegistry 自身不维护工具清单——清单由各模块的设计文档分别描述。

### 索引结构

一级索引按分组输出，格式如下：

- **分组头**：`**分组名** — (always loaded)` 或 `**分组名** — (deferred)`
- **常用工具**：`  - **工具名** (危险度): 行为描述`
- **延迟工具**：`  - 工具名 (危险度)`

危险度标记根据工具标记自动生成：只读工具标 `(read-only)`，破坏性工具标 `(destructive)`。分组头根据组内是否包含常用工具决定标注：全部延迟标 `(deferred)`，否则标 `(always loaded)`。

一级索引总长度有上限，超出时尾部截断。

### 工具发现

ToolSearch 是系统级工具发现入口。LLM 调用时传入关键词或工具名，注册中心匹配并返回对应工具的完整详情，后续注入到上下文中。关键词匹配支持工具名精确匹配和描述关键词模糊匹配。

### 安全边界

工具的权限检查不在工具层实现，而是由上游调用方在工具执行前通过权限引擎校验。工具本身通过运行时标记声明自身的安全属性（只读/破坏性/昂贵），供权限引擎和索引渲染使用。

### 自身工具

Tools 模块自身向 ToolRegistry 注册以下核心工具：

| 分组 | 工具 | 加载策略 | 所属模块 |
|------|------|---------|---------|
| bash | Bash | 始终加载 | tools |
| file_ops | Read、Write、Edit、Grep、Ls | 始终加载 | tools |
| git_ops | GitStatus、GitLog、GitCommit、GitPush、GitPull | 延迟加载 | tools |
| meta | ToolSearch、PermissionQuery | 始终加载 | tools |

Bash 工具的详细设计见 [bash-tool.md](bash-tool.md)。Git 操作组中状态和日志为只读，提交和推送为破坏性操作。

### 各模块注册的工具一览

以下工具由各模块自行注册到 ToolRegistry。定义和参数详见各模块设计文档。

| 注册模块 | 分组 | 工具 | 加载策略 |
|---------|------|------|---------|
| [Session](../session/README.md) | sessions | sessions_spawn、sessions_steer、sessions_kill | 始终加载 |
| [Skills](../skills/README.md) | skills | SkillTool | 始终加载 |
| [Skills](../skills/README.md) | skill_creator | SkillCreator | 延迟加载 |
| [IM Adapter](../im_adapter/README.md) | feishu_im | feishu_im_user_message、feishu_im_user_get_messages、feishu_im_user_get_thread_messages、feishu_search_user | 延迟加载 |
| [IM Adapter](../im_adapter/README.md) | feishu_calendar | feishu_calendar_event、feishu_calendar_event_attendee、feishu_calendar_freebusy、feishu_calendar_calendar | 延迟加载 |
| [IM Adapter](../im_adapter/README.md) | feishu_task | feishu_task_task、feishu_task_tasklist、feishu_task_comment、feishu_task_subtask | 延迟加载 |
| [IM Adapter](../im_adapter/README.md) | feishu_bitable | feishu_bitable_app、feishu_bitable_app_table、feishu_bitable_app_table_record、feishu_bitable_app_table_field、feishu_bitable_app_table_view | 延迟加载 |
| [IM Adapter](../im_adapter/README.md) | feishu_doc | feishu_doc_comments、feishu_doc_media、feishu_search_doc_wiki | 延迟加载 |
| [IM Adapter](../im_adapter/README.md) | feishu_drive | feishu_drive_file | 延迟加载 |
| [IM Adapter](../im_adapter/README.md) | feishu_sheet | feishu_sheet | 延迟加载 |

> 飞书工具专注 IM 平台的领域操作（消息收发、日历管理、任务协作、文档编辑等）。所有飞书工具默认延迟加载，LLM 通过工具发现按需获取详情。

## 数据流

### 注册与注入

```
系统启动
  → 各模块初始化时调用 ToolRegistry.register() 注册各自工具
    → system prompt 构建时调用索引构建
      → 生成分组索引字符串
        → 注入 system prompt 的工具区
          → LLM 在对话初始化时看到所有工具
```

### 工具调用

```
LLM 选择工具并生成调用参数
  → agent 运行时解析工具调用
    → 权限引擎校验
      → 通过：执行工具调用，返回结果
      → 拒绝：返回权限错误
```

### 工具发现

```
LLM 需了解延迟工具详情
  → 调用 ToolSearch（关键词或工具名）
    → 注册中心匹配
      → 返回工具完整详情
        → 注入当前上下文
          → LLM 在后续对话中使用该工具
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| 所有注册工具的模块 | 启动时调用 ToolRegistry::register() 注册自身工具 |
| System Prompt 构建器 | 调用索引构建注入工具区 |
| Agent 运行时 | 调度工具调用 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| 权限引擎 | 工具执行前校验 |
| Skill 系统 | SkillTool 桥接 skill 注册表 |

### 无关

| 模块 | 说明 |
|------|------|
| Processor Chain | 工具不参与消息出站处理 |
| Renderer | 工具不参与平台渲染 |
