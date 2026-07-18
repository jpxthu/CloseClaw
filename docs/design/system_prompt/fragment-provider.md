# PromptFragmentProvider

## 概述

PromptFragmentProvider 是系统提示词片段提供者的统一 trait，定义在 [common 模块](../common/core-traits.md#promptfragmentprovider)。将静态层各数据来源（bootstrap 文件、ToolRegistry、MEMORY.md）抽象为一致的接口。System Prompt Builder 通过收集已注册的 Provider 并依次调用，组装静态层内容，不再硬编码各来源的特定接口。

## 架构

### Trait 接口

PromptFragmentProvider 的完整接口定义见 [core-traits](../common/core-traits.md#promptfragmentprovider)。本文档聚焦 system_prompt 模块对 Provider 的编排逻辑。

### FragmentContext

Builder 在构建时提供的上下文，传递给每个 Provider。定义见 [shared-types](../common/shared-types.md#fragmentcontext)。

### PromptFragment

单个 Provider 产出的片段。定义见 [shared-types](../common/shared-types.md#promptfragment)。

### Provider 注册与 Builder 组装

1. Builder 持有 Provider 列表（启动时注册）
2. 构建触发（session 创建 / 恢复 / compaction）
3. 按优先级排序 Provider
4. 逐个请求片段（传入 FragmentContext）
5. 跳过返回空的 Provider
6. 按序拼接所有 Fragment 的内容
7. 产出静态层完整文本

### 三个标准 Provider

| Provider | priority | 来源 | 产出 |
|----------|----------|------|------|
| BootstrapFragmentProvider | 1 | bootstrap 文件（Bootstrap Loader） | 多文件聚合为单个 Fragment（内部以 `## 文件名` 分隔） |
| ToolsFragmentProvider | 2 | ToolRegistry | ToolsSection |
| MemoryFragmentProvider | 3 | MEMORY.md | MemorySection |

BootstrapFragmentProvider 将多文件内容聚合到单 Fragment 中，每文件以 `## 文件名` 为 Section 标题，按文件名排序。MemoryFragmentProvider 根据 FragmentContext 中的 bootstrap_mode 判断——Minimal 模式（子 Agent 会话）返回空 Fragment，不暴露长期记忆；Full 模式（主 Agent 会话）读取 MEMORY.md 生成 MemorySection。各 Provider 产出与原 Builder 中硬编码的文本完全一致——仅抽象了获取方式，不改变输出内容。

### Section 级缓存

Builder 在请求片段前检查缓存键命中。缓存策略不变：

- 文件变更 → 对应文件型 Section 缓存失效
- `/clear` → 全部缓存失效
- Session 恢复 → 强制跳过缓存，全部重新生成
- Compaction → 触发重建，全部重新生成

## 数据流

1. SessionManager 触发构建
2. Builder 构建 FragmentContext（agent_id + bootstrap_mode + bootstrap_dir）
3. 按优先级遍历注册的 Provider：
   - **BootstrapFragmentProvider**：检查缓存命中（基于 bootstrap 文件修改时间）→ Bootstrap Loader 读文件 → 聚合多文件为单 Fragment → 产出 Fragment（无 workspace 目录时返回空）
   - **ToolsFragmentProvider**：ToolRegistry 生成分组索引 → 产出 Fragment
   - **MemoryFragmentProvider**：检查缓存命中（基于 MEMORY.md 修改时间）→ 读 MEMORY.md → 产出 Fragment（文件缺失时返回空；Minimal 模式返回空）
4. 跳过返回空的 Provider
5. 按序拼接所有产出 Fragment 的内容
6. 写入 ConversationSession 的 system prompt 字段

### 兜底

兜底规则与 PromptFragmentProvider 接口契约一致（见 [core-traits](../common/core-traits.md#promptfragmentprovider)）。所有 Provider 均返回空时使用默认 prompt，无 workspace 目录时静态层仅含工具片段。

## 模块关系

### 上游

- **SessionManager**：在 session 创建、archive 恢复、compaction 完成时触发构建，传入 agent_id（builder 据此查询 agent 配置确定 bootstrap_dir 和 bootstrap_mode）。Builder 启动时持有 ToolRegistry 引用。
- **ConversationSession**：提供 agent 的 bootstrap_mode（Minimal/Full）。

### 下游

- **Bootstrap Loader**：BootstrapFragmentProvider 调用，提供 bootstrap 文件内容。
- **ToolRegistry**：ToolsFragmentProvider 调用，生成工具分组索引。
- **MEMORY.md**：MemoryFragmentProvider 直接读取，作为长期记忆 Section 来源。

### 无关

- **动态层**：PromptFragmentProvider 是静态层的抽象。动态层由 ConversationSession 直接组装，不走此 trait。
- **追加区**：追加区由 `/system` 指令管理，不走此 trait。
- **Cache Adapter**：静态层整体文本在构建完成后传给 Cache Adapter。Builder 和 Provider 层面的 Section 级缓存与 API 侧 KV Cache 是独立的两层。
