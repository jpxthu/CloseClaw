# Session 注入

## 概述

描述新 session 创建时，系统如何将 bootstrap 文件、工具列表、skill 列表和 memory 文件组装成 system prompt 并注入会话。注入链路是 session 生命周期中"创建"阶段的核心环节。

## 架构

### 注入入口

注入流程由 SessionManager 在 find_or_create 中触发，分为三个决策分支：

- **命中 active session**：直接返回，不重新注入。
- **命中 archived session**：从存储恢复后重新走完整注入流程，确保 system prompt 内容与最新 bootstrap 文件一致。
- **新 session**：走完整注入链路。

注入链路的核心是 system prompt builder 的 build_from_workspace，它接收已加载的 bootstrap 文件、ToolRegistry 引用和 SkillRegistry 引用，组装生成最终 system prompt。

### 注入的四个 Section

System Prompt 的 Section 类型定义见 [system-prompt/README.md](../system-prompt/README.md)。注入链路中 build_from_workspace 对四个 Section 的组装行为：

- **RoleSection**：调用 load_bootstrap_files 加载 bootstrap 文件，按固定顺序排列（AGENTS.md 排在首位）。Minimal 模式加载 AGENTS/SOUL/IDENTITY/USER/TOOLS，Full 模式额外加载 BOOTSTRAP 和 MEMORY。HEARTBEAT.md 不在注入范围。
- **ToolsSection**：通过 ToolRegistry 生成工具分组索引（常用工具含行为描述，延迟工具仅含名称和危险度标记）。
- **SkillListingSection**：通过 DiskSkillRegistry.generate_listing(agent_id) 生成 skill 摘要清单，按优先级排序（来源优先级 → 名字字母序）。
- **MemorySection**：读取 MEMORY.md，走 session 级文件缓存，命中缓存则跳过读取。

四个 Section 拼接后写入 ConversationSession 的 system_prompt 字段。

### 静态层与动态层的注入行为

静态层和动态层的边界定义见 [system-prompt/README.md](../system-prompt/README.md)。注入链路的行为：

- **静态层**（RoleSection + ToolsSection + SkillListingSection + MemorySection）：session 创建时注入，写入 ConversationSession 的 system_prompt 字段，进入 checkpoint 持久化。
- **动态层**（ChannelContext + SessionState + GitStatus + AppendSection）：每次 API 请求时注入，不持久化，不改变 session 的 system_prompt 字段。

### 无 Workspace 的 Session

当 session 没有对应 workspace 目录时，不加载 bootstrap 文件，但仍可通过 ToolRegistry 和 SkillRegistry 生成工具列表和 skill 列表。SkillListingSection 不依赖 workspace 目录，直接从注册中心获取。system prompt 仅包含 ToolsSection 和 SkillListingSection。

### Section 失败处理

单个 Section 组装失败时，该 Section 不参与拼接（跳过），其余 Section 继续。不阻断整条注入链路。

### Skill 热更新

skill 文件变更时，SkillRegistry 通知 SessionManager 重新走 build_from_workspace（重建 system prompt，无 workspace 时只重建 ToolsSection 和 SkillListingSection），生成新的 system prompt 替换旧的。不依赖消息 ID 定位（skill listing 直接拼接在 system prompt 中，不是独立消息）。

## 数据流

### 新 Session 创建

```
用户消息到达
  →
  SessionManager.find_or_create
    ├─ active session 命中 → 直接返回（不重新注入）
    ├─ archived session 命中 → 恢复 → 重新注入（保证 prompt 最新）
    └─ 新 session
        ├─ workspace 存在？
        │   ├─ 是 → 初始化 workdir = {config_dir}/workspaces/{agent_id}/{user_id}/
        │   │      → load_bootstrap_files（按 Minimal/Full 模式加载）
        │   │      → build_from_workspace
        │   │         ├─ 组装 RoleSection（bootstrap 文件内容）
        │   │         ├─ 组装 ToolsSection（ToolRegistry 生成工具描述）
        │   │         ├─ 组装 SkillListingSection（SkillRegistry 生成 skill 列表）
        │   │         ├─ 组装 MemorySection（MEMORY.md，走缓存）
        │   │         └─ 拼接生成 system prompt
        │   │      → ConversationSession.with_system_prompt
        │   │
        │   └─ 否 → 组装 ToolsSection + SkillListingSection（跳过 bootstrap）
        │          → ConversationSession.with_system_prompt
        │
        └─ 写入 SessionManager 的运行时映射
           → 返回 session_id
```

### 每次 API 请求时的动态层注入

```
API 请求到达
  →
  build_channel_context（当前消息的 channel/sender 信息）
  build_session_state（turnCount 等运行时状态）
  →
  动态层追加到静态 system_prompt 后
  →
  发送 LLM 请求
```

动态层不进 checkpoint，不改变 session 的 system_prompt 字段。

### Session 恢复时的重新注入

从 checkpoint 重建 ConversationSession 后，重新走完整注入流程（而非恢复旧的 system_prompt），确保内容与最新 bootstrap 文件一致。

```
archived session 被访问
  →
  从 checkpoint 重建 ConversationSession
  →
  SessionManager 重新走 build_from_workspace（不恢复旧的 system_prompt）
  →
  新的 system_prompt 替换旧的
```

## 模块关系

### 上游

- **SessionManager**：session 生命周期协调者，在 find_or_create 中调用 system prompt builder 完成注入，同时初始化 session 工作目录字段。持有 ToolRegistry 和 SkillRegistry 引用，作为注入的数据来源。

### 下游

- **ToolRegistry**：提供 build_tools_section，生成工具的 name + description + 危险度标记。
- **SkillRegistry**：提供 generate_listing，生成可用 skill 列表（按优先级 + 字母序排序）。
- **Bootstrap Loader**：提供 load_bootstrap_files，按 Minimal/Full 模式加载 bootstrap 文件集合。

### 无关

- **LLM Provider**（无调用关系）：注入完成后的 system prompt 通过 ConversationSession 传递给 LLM provider，注入链路本身不调用 provider。
- **Compaction 模块**（无调用关系）：compaction 时 skill listing 的保护策略在 compaction 模块定义，注入链路不参与压缩逻辑。
