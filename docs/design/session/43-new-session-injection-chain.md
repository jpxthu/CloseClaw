# 43 新 Session 启动注入链路

## 职责

新 session 创建时，协调 bootstrap 文件、系统提示词 sections、工具列表、skill 列表的完整组装，生成 system prompt 并注入 session。由 SessionManager 统筹，system_prompt builder 执行组装。

## 设计意图

**问题**：当前生产代码中，`SessionManager` 创建新 session 时直接拼接 bootstrap 文件，跳过了 system_prompt builder 的完整组装能力。导致：
- skill 列表从未进入 system prompt（设计早已定义但代码未落地）
- 工具描述从未进入 system prompt（builder 中是空壳）
- bootstrap 文件没有经过 section 化处理，缺少统一的结构标记

**目标**：打通注入链路，让所有静态内容（bootstrap、工具、skill、memory）通过 builder 统一组装，一次性生成完整 system prompt。

## 核心思路

**组装入口唯一**：系统提示词 builder 的完整组装路径是生成 system prompt 的唯一入口，不允许多路径拼接。所有静态 section 在此处组装，动态 section 在每次 API 请求时追加。

**基础设施与 workspace 解耦**：工具注册表和 skill 注册表是 daemon 级的平台基础设施，独立于 workspace 路径。即使 workspace 目录不存在，session 仍可获得工具列表和 skill 列表——跳过的是 bootstrap 文件（AGENTS、SOUL 等），不跳过 registry 生成的内容。

**静态/动态边界**：system prompt 分为静态区和动态区，用 `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 标记分隔。静态区在 session 创建时组装并存入 session checkpoint；动态区（渠道上下文、会话状态）在每次 API 请求时计算，不进 checkpoint。

**失败降级**：任何 section 渲染失败时，该 section 被跳过（不拼接任何内容），其他 section 继续组装。不抛异常、不阻断整棵构建树。与"失败返回空字符串"相比，避免多余空行和格式污染。

## 逻辑流转

### Session 创建流程

```
SessionManager 收到新消息
  │
  ├─ 1. 计算 session_id
  │
  ├─ 2. 检查活跃 session？→ 命中则直接返回
  │
  ├─ 3. 尝试从归档恢复？→ 命中则恢复后仍走注入流程（保证内容新鲜）
  │
  └─ 4. 创建新 session
        │
        ├─ workspace 目录存在？
        │   │
        │   ├─ 是 → 加载 bootstrap 文件（按 boot_mode 决定 Minimal/Full）
        │   │      └─ 走完整组装路径，生成全部 sections
        │   │
        │   └─ 否 → 跳过 bootstrap 文件加载
        │          └─ 走最小化组装路径
        │             └─ 仅组装 tools + skill listing（走 registry 生成）
        │                ├─ bootstrap 文件：跳过
        │                └─ memory：跳过（无 workspace 路径）
        │
        ├─ 组装结果写入 ConversationSession
        │
        └─ 返回 session_id
```

### Skill Hot Reload 时的 system prompt 更新

skill 文件发生变更时，通过文件系统监听触发更新链路：文件变更事件 → skill registry 失效 → 通知 SessionManager → 对持有的活跃 session 重新走完整组装路径，生成新的 system prompt 替换旧值。

由于 skill listing 直接拼接在 system prompt 中（而非作为独立消息），更新方式是整体替换 system prompt 而非定位替换单条消息。

### 完整组装路径内部顺序

```
builder 接收 workspace 路径、动态 sections、skill registry、agent_id 和追加内容
  │
  ├─ 1. 加载 bootstrap 文件（经统一加载器，按规定的文件优先级排序）
  │     └─ Minimal: AGENTS → SOUL → IDENTITY → USER → TOOLS
  │     └─ Full: 上述 + BOOTSTRAP + MEMORY
  │
  ├─ 2. 组装 RoleSection（bootstrap 文件拼接）
  │
  ├─ 3. 组装 MemorySection（MEMORY.md 内容，走缓存）
  │
  ├─ 4. 组装 ToolsSection（registry 生成，描述所有工具的一级索引）
  │
  ├─ 5. 组装 SkillListingSection（registry 生成，按 source 优先级 + 名称排序）
  │
  ├─ 6. 拼接待追加内容（`/system` 命令产生的动态追加，每次调用覆盖前一次，若存在）
  │
  └─ 7. 输出完整 system prompt string
        └─ 拼接格式：[RoleSection] [MemorySection] [ToolsSection] [SkillListingSection]
           __SYSTEM_PROMPT_DYNAMIC_BOUNDARY__ [append_content]
```

### 每次 API 请求时的动态注入

动态 section 有两个注入层级：

**Session 启动时**：动态 sections 列表为空。此时 session 刚创建，还没有当前消息的渠道上下文和会话状态，仅注入静态 sections（bootstrap、tools、skill、memory）。

**每次 API 请求时**：根据当前消息构建渠道上下文（channel type、sender 信息等）和会话状态（轮次计数等），作为动态 sections 追加到静态 prompt 之后。动态内容不改变 session 存储的 system_prompt 字段，不进 checkpoint。

### 恢复 session 的特殊处理

从归档恢复的 session 不沿用旧的 system prompt，而是重新走完整注入流程。SessionCheckpoint 中不保存 tools 和 skill listing，恢复后必须重新生成。同时 bootstrap 文件可能已经变更，用旧内容会导致行为不一致。

## 与其他模块的关系

| 模块 | 关系 | 传递的信息 |
|------|------|-----------|
| system_prompt/builder | **下游**：SessionManager 调用 builder 生成 prompt | 传入 workspace 路径、registry 引用、agent_id；返回完整 system prompt 字符串 |
| system_prompt/sections | **下游**：定义 RoleSection、ToolsSection、SkillListingSection 等 section 类型 | builder 按 section 类型组装，决定拼接顺序和格式 |
| tools/registry | **上游**：提供工具描述生成能力 | 调用 build_tools_section 获取工具一级索引（name + detail + 危险度标记） |
| skills/disk | **上游**：提供 skill 列表生成能力 | 调用 generate_listing 获取可用 skill 列表（name + description + whenToUse） |
| bootstrap/loader | **上游**：加载 workspace 中的 bootstrap 文件 | 返回按文件优先级排序的文件列表，builder 直接消费 |
| llm/session（ConversationSession） | **下游**：持有生成的 system prompt | builder 输出赋值给 ConversationSession 的 system_prompt 字段 |
| session/storage（checkpoint） | **下游**：持久化 session 状态 | system_prompt 静态部分写入 checkpoint，动态部分不写入，tools 和 skill listing 不单独保存 |
| daemon | **上游**：在启动时初始化 ToolRegistry（注册全部内置工具）和 DiskSkillRegistry | registry 实例在 daemon 启动阶段创建，SessionManager 在构建时接收引用 |

**无关模块**：工具调用逻辑（tools 模块的 call 路径）、skill 执行模型、compaction 流程、heartbeat 触发——这些模块虽然消费 system prompt 中的内容，但不参与注入链路的组装过程。

## 关键决策

### 1. 为什么完整组装路径是唯一入口

**选择**：所有 system prompt 内容只能通过 builder 的完整组装路径生成，不允许多路径拼接。
**不选**：让 SessionManager 单独调用 load_bootstrap_files、build_tools_section、generate_listing，再自行拼接。

直接拼接的问题是：每次新增 section 类型时，SessionManager 需要同步修改拼接逻辑，且无法保证组装顺序和格式的一致性。统一入口后，builder 负责所有 section 的加载、排序、拼接，SessionManager 只做调度。

### 2. 为什么 tool_registry 和 skill_registry 是 daemon 级而非 workspace 级

**选择**：两个 registry 在 daemon 初始化时创建，SessionManager 持有引用，与 workspace 路径解耦。
**不选**：registry 绑定到 workspace 路径，workspace 不存在时 registry 也不可用。

工具和 skill 是平台能力，不依赖特定 workspace。即使 session 没有 workspace（如 `/system` 命令触发的纯会话），用户仍需要工具列表来执行操作。将 registry 提升到 daemon 层后，无 workspace 的 session 也能通过最小化组装路径获得 tools + skill listing。

### 3. 为什么 section 失败时跳过而非抛异常

**选择**：单个 section 渲染失败时跳过该 section，其他 section 继续。
**不选**：抛异常终止整个 system prompt 构建，或返回空字符串拼接进结果。

跳过而非抛异常的理由：system prompt 是由多个独立 section 组合而成，一个 section 的失败不应污染全局。返回空字符串的代价是在最终 prompt 中留下多余空行，跳过则完全消除影响。参考 Claude Code 的做法：compute 返回 null 时 filter 移除，section 不贡献内容。

### 4. 为什么恢复 session 时重新走注入流程

**选择**：从归档恢复的 session 丢弃旧 system prompt，重新走完整组装路径。
**不选**：恢复时沿用归档中保存的旧 system prompt。

归档的 system prompt 中 tools/skill listing 可能已过时（新工具上线、skill 更新），bootstrap 文件也可能被 owner 修改过。重新生成保证内容始终新鲜。代价是每次恢复多一次 builder 调用，但完整组装路径本身计算量小，可接受。

### 5. 缓存策略

system prompt 的每个 section 采用独立缓存，各自监听失效触发：

- **RoleSection**：监听 bootstrap 文件变更（AGENTS、SOUL、IDENTITY、USER、TOOLS），任一文件修改即失效
- **ToolsSection**：监听工具注册表变更，新增或移除工具时失效
- **SkillListingSection**：监听 skill 文件变更（hot reload 触发），替换为新 listing
- **MemorySection**：监听 MEMORY.md 文件写入，每次写入后重新加载

动态 sections（ChannelContext、SessionState）不缓存，每次 API 请求重新计算。任何 section 渲染失败时，仅该 section 跳过，缓存不受污染。

### 6. 工具注入的 eager 模式

当前阶段，所有工具以 eager 模式全部进入 system prompt，包括名称、功能描述和危险度标记。后续如需按需加载（deferred 模式），依赖工具搜索能力的进一步完善，当前不在本文范围。

### 7. 为什么 skill listing 进入 system prompt 而非 prepend 到消息数组

**选择**：skill listing 作为 SkillListingSection 进入 system prompt，不单独 prepend 到消息数组最前面。
**不选**：像 Claude Code 一样作为 system-reminder 消息 prepend。

Claude Code 将 skill list 作为独立消息 prepend，让模型在处理每条历史消息时都看到。这个设计更稳定（不受 compaction 影响），但增加了消息数组维度的管理复杂度。CloseClaw 选择进入 system prompt，实现更简单，代价是 compaction 时需要特殊保护 skill listing section（与 bootstrap 同等保护）。当前阶段优先简单实现，后续如发现 compaction 导致 skill 信息丢失过多，再评估是否改为 prepend 方案。
