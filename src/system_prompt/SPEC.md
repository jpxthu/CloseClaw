# `src/system_prompt` 模块规格说明书

> 本文档描述模块**当前实际行为**，代码与文档不一致时以代码为准。

---

## 1. 模块概述

提供分层 System Prompt 构建系统。

覆盖链（override → agent → custom → default）加 append 始终追加在最末。静态 section 基于 mtime 被动失效（无主动文件监听）；动态 section 每次请求重新渲染。Workspace 路径下 IDENTITY.md/SOUL.md/MEMORY.md 自动参与 RoleSection 拼接。

权限校验由调用方负责；不主动监听文件变化；append 内容请求结束后自动清除。

---

## 2. 公开接口

### Prompt 构建

| 接口 | 功能 |
|------|------|
| `build_system_prompt` | 组装完整 system prompt，按覆盖链优先级拼接各 section |
| `build_from_workspace(workspace_root, dynamic_sections, skill_info)` | 从 workspace 路径加载 bootstrap 文件（IDENTITY.md / SOUL.md / MEMORY.md）构建 prompt，skill_info 为 `Some((registry, agent_id))` 时注入 SkillListingSection |
| `set_override_prompt` | 设置最高优先级覆盖 prompt |
| `set_agent_prompt` | 设置 agent 级 prompt（覆盖链第二层） |
| `set_custom_prompt` | 设置用户自定义 prompt（覆盖链第三层） |

### Section 缓存

| 接口 | 功能 |
|------|------|
| `get_cached_section` | 获取缓存 section（mtime 变化则失效） |
| `put_cached_section` | 写入 section 缓存（含 mtime） |
| `read_file_section` | 读取文件内容 + mtime（文件驱动 section 的底层 I/O 原语） |
| `load_cached_file_section` | mtime 未变时返回缓存，否则重新加载并写入缓存 |
| `invalidate_section` | 手动失效单个 section |
| `invalidate_all_sections` | 失效所有 section |
| `APPEND_SECTION_MAX_LEN` | append 内容最大字符数常量（500） |

### Append Section（请求级）

| 接口 | 功能 |
|------|------|
| `set_append_section` | 设置追加内容，超长自动截断并返回警告 |
| `get_append_section` | 获取当前追加内容 |
| `clear_append_section` | 清除追加内容（请求结束后调用） |

### Workdir

| 接口 | 功能 |
|------|------|
| `set_workdir` | 设置工作目录，返回 WorkdirContext（含路径、git 状态） |
| `get_workdir` | 获取当前工作目录 |
| `clear_workdir` | 清除工作目录 |
| `build_git_status` | 为当前 workdir 构建内嵌 git 状态字符串 |

### 斜杠指令

| 接口 | 功能 |
|------|------|
| `handle_system_command` | 处理 `/system`（显示或设置追加内容） |
| `handle_cd_command` | 处理 `/cd`（切换工作目录） |
| `handle_pwd_command` | 处理 `/pwd`（返回当前工作目录） |
| `handle_git_command` | 处理 `/git`（status 返回内嵌 gitStatus，其他委托系统 git） |

---

## 3. 架构与结构

### 子模块划分

| 子模块 | 职责 |
|--------|------|
| `builder` | 拼接各 section 为完整 prompt 字符串 |
| `sections` | Section 定义、缓存管理、append 管理 |
| `workdir` | 工作目录上下文及 git 状态 |
| `slash_commands` | `/system`、`/cd`、`/pwd`、`/git` 指令处理 |

### Section 分类

- **静态（缓存）**：RoleSection、WorkspaceSection、ToolsSection、MemorySection、HeartbeatSection、SkillListingSection
- **动态（每次重建）**：ChannelContext、SessionState、AppendSection、GitStatus

`SkillListingSection` 在 `build_from_workspace` 中从 `DiskSkillRegistry::generate_listing` 注入，位于 ToolsSection 之后、dynamic_sections 之前；内容为空时不渲染。

### 覆盖优先级（从高到低）

1. `overrideSystemPrompt`
2. `agentSystemPrompt`
3. `customSystemPrompt`
4. `defaultSystemPrompt`（默认：`"You are CloseClaw, a helpful AI assistant."`）
5. `appendSection`（始终追加在最末）

### Workspace 文件拼接约定

**Builder 架构**：`build_from_workspace` 通过 `load_bootstrap_files`（session::bootstrap::loader ⚠️ 架构唯一入口）加载 bootstrap 文件，按 `BootstrapMode` 决定文件集合：
- Minimal：AGENTS.md / SOUL.md / IDENTITY.md / USER.md / TOOLS.md
- Full：上述 + BOOTSTRAP.md / MEMORY.md

**HEARTBEAT.md 不属于 bootstrap**：HEARTBEAT.md 由 cron prompt 指示 agent 按需读取，不在任何 bootstrap 模式中，也不通过 `load_bootstrap_files` 加载。

`SOUL.md` 内容合并至 IDENTITY.md 生成的 RoleSection 末尾，而非独立 section。`MEMORY.md` 使用 mtime-aware 缓存。

### 数据类型

- `Section` — 9 种子类型的 section 枚举，分静态/动态两大类；含 `is_cacheable()`、`name()`、`render()` 三个公开方法
- `WorkdirContext` — 含 path、has_git、branch、recent_changes 四个字段
- `APPEND_SECTION_MAX_LEN` — append 内容最大字符数（500）
