# PromptFragmentProvider

## 概述

PromptFragmentProvider 是系统提示词片段提供者的统一 trait，定义在 [common 模块](../common/core-traits.md#promptfragmentprovider)。将静态层各数据来源（Bootstrap 文件、ToolRegistryQuery、SkillListingProvider、MEMORY.md）抽象为一致的接口。System Prompt Builder 通过收集已注册的 Provider 并依次调用，组装静态层内容，不再硬编码各来源的特定接口。

## 架构

### Trait 接口

PromptFragmentProvider 的完整接口定义见 [core-traits](../common/core-traits.md#promptfragmentprovider)。本文档聚焦 system_prompt 模块对 Provider 的编排逻辑。

### FragmentContext

Builder 在构建时提供的上下文，传递给每个 Provider。定义见 [shared-types](../common/shared-types.md#fragmentcontext)。与 Session 类型相关的字段：

- `session_role`：主 Agent Session / 子 Session（由 SessionManager 传入）
- `bootstrap_mode`：身份加载模式，仅主 Agent Session 决定自定义引导指令取舍
- `no_bootstrap`：Agent 是否显式声明无 Bootstrap 文件
- `bootstrap_dir_present`：Bootstrap 文件目录是否存在（Builder 在构建时检查 `agentDir`）

其余字段：`agent_id`（可见范围）、`bootstrap_dir`（Bootstrap 文件路径）、`activated_skills`（已激活条件技能）、`tool_registry`（ToolRegistryQuery 引用）。

各 Provider 将「`no_bootstrap` 为真或 `bootstrap_dir_present` 为假」统一视为「无 Bootstrap 文件的 Session」。类型判定规则（含降级告警）见 [static-layer.md §Session 类型与 Bootstrap 文件加载](static-layer.md)。

### PromptFragment

单个 Provider 产出的片段。定义见 [shared-types](../common/shared-types.md#promptfragment)。

### Provider 注册与 Builder 组装

1. Builder 持有 Provider 列表（启动时注册）
2. 构建触发（Session 创建 / 归档恢复 / 压缩完成 / Owner 清除追加指令）
3. 按优先级排序 Provider
4. 逐个请求片段（传入 FragmentContext）
5. 跳过返回空的 Provider
6. 按序拼接所有 Fragment 的内容
7. 产出静态层完整文本

构建在异步执行环境中完成：各 Provider 的调用（含经 SkillListingProvider 获取技能清单、经 ToolRegistryQuery 生成工具索引）均在异步任务中执行，不同步阻塞运行时。

### 四个标准 Provider

| Provider | priority | 来源 | 产出 |
|----------|----------|------|------|
| BootstrapFragmentProvider | 1 | bootstrap 文件（Bootstrap Loader） | 多文件聚合为单个 Fragment（内部以 `## 文件名` 分隔） |
| ToolsFragmentProvider | 2 | ToolRegistryQuery（工具名单经 AgentToolsConfigQuery 取得） | ToolsSection |
| SkillsFragmentProvider | 3 | SkillListingProvider（底层数据源为 SkillRegistry） | SkillsSection |
| MemoryFragmentProvider | 4 | MEMORY.md | MemorySection |

四个标准 Provider 的 trait 实现分布在各自领域模块：BootstrapFragmentProvider 由 system_prompt 模块实现，ToolsFragmentProvider / SkillsFragmentProvider / MemoryFragmentProvider 分别由 tools / skills / memory 模块实现；system_prompt 模块负责注册与编排。

BootstrapFragmentProvider 将多文件内容聚合到单 Fragment 中，每文件以 `## 文件名` 为 Section 标题。文件注入顺序与 [static-layer.md](static-layer.md) 的固定顺序一致（AGENTS.md → SOUL.md → IDENTITY.md → USER.md → TOOLS.md → BOOTSTRAP.md；BOOTSTRAP.md 仅在主 Agent Session 且完整模式时参与该顺序）。每个 `##` 标题块是逻辑上的一个 Section，在 Provider 层面聚合到一个 PromptFragment 中传递。「无 Bootstrap 文件的 Session」不产出任何文件，规则见 [static-layer.md](static-layer.md) §Session 类型与 Bootstrap 文件加载。

ToolsFragmentProvider 经 [AgentToolsConfigQuery](../common/core-traits.md#agenttoolsconfigquery) 取得当前 Agent 的工具白/黑名单，作为参数传给 ToolRegistryQuery 生成按 Agent 过滤后的分组索引。

SkillsFragmentProvider 从 [SkillListingProvider](../common/core-traits.md#skilllistingprovider) 获取技能清单（生成规则见 [skills/skill-listing-injection](../skills/skill-listing-injection.md)）：初始构建使用不含未激活条件技能的清单；SP 重建时按 FragmentContext 的 `activated_skills` 纳入已激活条件技能，产出 SkillsSection。加载范围见 [static-layer.md](static-layer.md) §系统生成的 Section。

MemoryFragmentProvider 按 FragmentContext 判定的 Session 类型决定：主 Agent Session 读取 MEMORY.md 生成 MemorySection；子 Session 与「无 Bootstrap 文件的 Session」返回空 Fragment，不暴露长期记忆。加载范围与 MEMORY.md 路径规则见 [static-layer.md](static-layer.md) §系统生成的 Section。

各 Provider 产出与原 Builder 中硬编码的文本完全一致——仅抽象了获取方式，不改变输出内容。

### Section 级缓存

Builder 在请求片段前检查缓存键命中。缓存失效策略详见 [static-layer.md §Section 级缓存](static-layer.md)，本文件不重复定义。

## 数据流

1. SessionManager 触发构建，传入 agent_id、Session 角色（主/子）与身份加载模式覆写（如有）
2. Builder 根据 SessionManager 传入的 agent_id、Session 角色（主/子）、身份加载模式覆写（如有）与 `activated_skills`，查询 agent 配置（bootstrap 模式经 [AgentLookup](../common/core-traits.md#agentlookup)）获取 `agentDir`（写入 FragmentContext 的 `bootstrap_dir`）、`bootstrapMode` 默认值与 `noBootstrap`（有覆写时以覆写为准），检查 Bootstrap 文件目录是否存在，解析 Session 类型并构建 FragmentContext（agent_id + session_role + bootstrap_mode + no_bootstrap + bootstrap_dir + bootstrap_dir_present + activated_skills + tool_registry）。因目录不存在兜底时记录降级告警
3. 按优先级遍历注册的 Provider（各 Provider 按缓存键检查命中；工具/技能由注册表显式失效）：
   - **BootstrapFragmentProvider**：检查缓存命中（基于 Bootstrap 文件修改时间）→ Bootstrap Loader 读文件 → 聚合多文件为单 Fragment → 产出 Fragment（Session 类型为无 Bootstrap 文件时返回空）
   - **ToolsFragmentProvider**：经 ToolRegistryQuery 生成分组索引 → 产出 Fragment
   - **SkillsFragmentProvider**：经 SkillListingProvider 获取技能清单 → 初始构建使用不含未激活条件技能的清单，SP 重建时按 `activated_skills` 纳入已激活条件技能（不受 user-invocable 限制）→ 产出 Fragment（无可用技能时返回空）
   - **MemoryFragmentProvider**：检查缓存命中（基于 MEMORY.md 修改时间）→ 仅主 Agent Session（session_role=Main 且非无 Bootstrap 类型）读 MEMORY.md → 产出 Fragment（子 Session 与无 Bootstrap 类型的 Session 返回空；文件缺失时返回空）
4. 跳过返回空的 Provider
5. 按序拼接所有产出 Fragment 的内容
6. 写入 ConversationSession 的 System Prompt 字段

### 兜底

兜底规则与 PromptFragmentProvider 接口契约一致（见 [core-traits](../common/core-traits.md#promptfragmentprovider)）。所有 Provider 均返回空时使用默认 prompt；「无 Bootstrap 文件的 Session」静态层仅含工具片段和技能片段。

## 模块关系

### 上游

- **SessionManager**：在 Session 创建、归档恢复、压缩完成与 Owner 清除追加指令时触发构建，并在新 Session 创建 / spawn 子 Session 时确定其 Session 角色（主 Agent Session / 子 Session），随 agent_id、身份加载模式覆写（如有）与 `activated_skills` 一并传入。Builder 据此查询 agent 配置确定 bootstrap_dir、bootstrap_mode、no_bootstrap 并检查目录存在性；`tool_registry` 经 InjectionParams 注入。Builder 启动时持有 ToolRegistryQuery 引用。
- **Session 压缩流程（Compaction）**：压缩完成后通过回调触发静态层重建。

### 下游

- **Bootstrap Loader**：BootstrapFragmentProvider 调用，提供 bootstrap 文件内容。
- **ToolRegistryQuery**：ToolsFragmentProvider 调用，生成工具分组索引；工具名单经 AgentToolsConfigQuery 取得。
- **AgentToolsConfigQuery**：ToolsFragmentProvider 调用，取得当前 Agent 的工具白/黑名单。
- **SkillListingProvider**：SkillsFragmentProvider 调用，获取技能清单（底层数据源为 SkillRegistry）。
- **MEMORY.md**：MemoryFragmentProvider 按 memory 模块配置的规则文件路径读取，作为长期记忆 Section 来源。

### 无关

- **动态层**：PromptFragmentProvider 是静态层的抽象。动态层由 ConversationSession 直接组装，不走此 trait。
- **追加区**：追加区由 `/system` 指令管理，不走此 trait。
- **Cache Adapter**：静态层读取 Section 缓存，动态层与追加区即时组装；按边界标记确定静态区与动态区两个字段后传给 Cache Adapter（见 [kv-cache.md](kv-cache.md)）。Builder / Provider 层面的 Section 级缓存与 API 侧 KV Cache 是独立的两层。
