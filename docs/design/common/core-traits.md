# 核心 Trait

## 概述

核心 trait 是跨模块依赖注入的接口契约。每个 trait 在本文档中唯一定义其完整接口，各业务模块文档通过引用指向此处，不在自身文档中重复定义 trait 签名。

## 架构

### PromptFragmentProvider

**用途**：统一抽象 system prompt 静态层各数据来源（bootstrap 文件、ToolRegistry、DiskSkillRegistry、MEMORY.md），System Prompt Builder 通过收集已注册的 Provider 并依次调用组装静态层内容。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | Provider 的唯一名称，用于注册和日志 |
| 优先级 | 数值越小越靠前，决定片段在静态层中的排列顺序 |
| 片段生成 | 根据 [FragmentContext](shared-types.md#fragmentcontext) 产出 [PromptFragment](shared-types.md#promptfragment)。无内容时返回空（文件缺失、agent 无可见 skill 等），Builder 自动跳过 |
| 缓存键 | Section 级缓存的标识。不可缓存时返回空。文件型 Provider 基于文件修改时间生成键，注册表型 Provider 由各自注册表管理失效 |

四个标准 Provider（BootstrapFragmentProvider / ToolsFragmentProvider / SkillsFragmentProvider / MemoryFragmentProvider）的定义和 Provider 注册编排流程详见 [fragment-provider](../system_prompt/fragment-provider.md)。

兜底规则：所有 Provider 均返回空时，使用默认 prompt。无 workspace 目录时 BootstrapFragmentProvider 返回空，静态层仅含工具和 skill 片段。

### ToolRegistrar

**用途**：抽象各模块"我能注册工具"的接口契约。Tools 模块通过收集已注册的 Registrar 并依次调用其注册方法完成全局工具编排。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | Registrar 的唯一名称，用于日志和冲突报告 |
| 优先级 | 数值越小越靠前，决定各模块工具的注册顺序。同等优先级下注册顺序不保证 |
| 注册 | 接收 [ToolRegistry](#toolregistry) 引用，将本模块所有工具一次性注册。工具名冲突时中断启动 |

注册阶段的错误策略：
- **工具名冲突**：报告冲突工具名和双方 Registrar，中断启动
- **单个 Registrar 内部错误**：由 Registrar 自行处理（跳过无效工具并记录警告，不中断其他工具注册）。Registrar 整体注册失败则报告错误

四个标准 Registrar（CoreToolsRegistrar / SessionToolsRegistrar / SkillsToolsRegistrar / ImAdapterToolsRegistrar）的定义和编排流程详见 [tool-registrar](../tools/tool-registrar.md)。

### ToolRegistry（接口）

**用途**：全局工具注册中心接口。Tools 模块提供此接口的具体实现。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 注册工具 | 以工具名为键注册工具定义（名称、分组、摘要、行为描述、输入模式、运行时标记）。工具名冲突时拒绝注册 |
| 索引构建 | 按分组聚合已注册工具，生成一级索引字符串。常用工具展示名称和行为描述，延迟加载工具仅展示名称和危险度标记 |
| 工具查询 | 按工具名返回完整详情；按分组名返回该组下所有工具名 |
| 冻结 | 标记注册完成，拒绝后续注册调用。冻结后仅允许查询操作 |

具体实现和工具注册编排流程详见 [tools 模块](../tools/README.md)。

## 数据流

core-traits 本身不参与运行时数据流。trait 接口在依赖注入时绑定实现，各业务模块通过 trait 接口交互而非直接依赖实现模块。

### PromptFragmentProvider 注册与调用

1. 系统启动 → System Prompt Builder 收集所有 Provider 实现者 → 按优先级排序
2. 构建触发（session 创建/恢复/compaction）
3. Builder 构建 [FragmentContext](shared-types.md#fragmentcontext)（agent 标识 + bootstrap 模式 + 工作目录）
4. 按优先级遍历 Provider → 检查缓存（命中则复用，未命中则调用片段生成）→ 跳过返回空的 → 按序拼接产出 [PromptFragment](shared-types.md#promptfragment)
5. 写入 ConversationSession 的 system prompt 字段

缓存由 Builder 内部管理，详细缓存策略和失效规则见 [fragment-provider](../system_prompt/fragment-provider.md)。

### ToolRegistrar 注册与编排

1. 系统启动 → Tools 模块收集所有 ToolRegistrar 实现者 → 按优先级排序
2. 依次调用各 Registrar → 向 [ToolRegistry](#toolregistry) 注册工具 → 注册完成 → ToolRegistry 冻结
3. 后续流程（索引构建、工具发现、system prompt 注入）不变

## 模块关系

- **上游**：无（common 不依赖任何其他模块，是纯定义基底层）
- **下游**：
  - **system_prompt**（实现 BootstrapFragmentProvider，System Prompt Builder 收集所有 Provider 并触发生成）
  - **tools**（实现 ToolsFragmentProvider 和 CoreToolsRegistrar，提供 ToolRegistry 具体实现，收集 ToolRegistrar 实现者并编排调用）
  - **session**（实现 SessionToolsRegistrar）
  - **skills**（实现 SkillsFragmentProvider 和 SkillsToolsRegistrar）
  - **memory**（实现 MemoryFragmentProvider）
  - **im_adapter**（实现 ImAdapterToolsRegistrar）
- **无关**：Processor Chain（不参与 trait 接口定义或 DI 绑定）
