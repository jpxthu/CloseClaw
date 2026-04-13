# `src/system_prompt` 模块规格说明书

> 本文档描述模块**当前实际行为**，代码与文档不一致时以代码为准。

---

## 1. 模块职责

提供分层 System Prompt 构建系统，支持：
- 静态区（文件驱动，可缓存）
- 动态区（每次请求重建）
- 工作目录上下文与 gitStatus
- `/system`、`/cd`、`/pwd`、`/git` 斜杠指令

---

## 2. 公开 API

```rust
// === 核心构建 ===

/// 构建完整 system prompt，按优先级组装各 section
pub fn build_system_prompt(sections: Vec<Section>) -> String

/// 使用 workspace 标准路径构建 prompt
pub fn build_from_workspace<P: AsRef<Path>>(
    workspace_root: P,
    dynamic_sections: Vec<Section>,
) -> String

// === 覆盖 prompt（优先级链） ===

/// 最高优先级：覆盖整个 prompt
pub fn set_override_prompt(prompt: Option<String>)
/// 第二优先级：agent 级 prompt
pub fn set_agent_prompt(prompt: Option<String>)
/// 第三优先级：用户自定义 prompt
pub fn set_custom_prompt(prompt: Option<String>)

// === Section 缓存 ===

/// 手动失效单个 section
pub fn invalidate_section(name: &str)
/// 失效所有 section
pub fn invalidate_all_sections()
/// 获取缓存的 section（如 mtime 未变）
pub fn get_cached_section(name: &str, current_mtime: Option<u64>) -> Option<String>

// === Append Section（请求级） ===

/// 设置追加内容，超长返回截断警告
pub fn set_append_section(text: String) -> Option<String>
/// 获取当前追加内容
pub fn get_append_section() -> Option<String>
/// 清除追加内容（请求结束后调用）
pub fn clear_append_section()

// === Workdir ===

/// 设置工作目录，返回元数据
pub fn set_workdir(path: String) -> WorkdirContext
/// 获取当前工作目录（如未设置返回 None）
pub fn get_workdir() -> Option<String>
/// 清除工作目录
pub fn clear_workdir()
/// 读取当前 workdir 的 git branch 和未提交变更数，返回格式化字符串供 GitStatus Section 渲染；非 git repo 返回 None
pub fn build_git_status() -> Option<String>

// === 斜杠指令 ===

pub fn handle_system_command(args: &str) -> SlashCommandResult
pub fn handle_cd_command(args: &str) -> SlashCommandResult
pub fn handle_pwd_command() -> SlashCommandResult
pub fn handle_git_command(args: &str) -> SlashCommandResult

// === 常量 ===
pub const APPEND_SECTION_MAX_LEN: usize = 500
```

---

## 3. Section 定义

### 3.1 静态区（可缓存，文件驱动）

| Section | 类型 | 内容来源 | 缓存方式 |
|---------|------|---------|---------|
| `RoleSection` | 静态 | IDENTITY.md + SOUL.md 内容拼接 | mtime |
| `MemorySection` | 静态 | MEMORY.md 内容 | mtime |
| `HeartbeatSection` | 静态 | HEARTBEAT.md 内容 | mtime |
| `WorkspaceSection` | 静态 | `Section` 枚举存在，builder **未注入** | — |
| `ToolsSection` | 静态 | `Section` 枚举存在，builder **未注入** | — |

### 3.2 动态区（每次请求重建）

| Section | 类型 | 内容来源 |
|---------|------|---------|
| `ChannelContext` | 动态 | `chat_name` + `sender_id` + `timestamp` |
| `SessionState` | 动态 | `turn_count` + `pending_tasks` |
| `AppendSection` | 动态 | `/system` 命令追加内容 |
| `GitStatus` | 动态 | 工作目录 git 状态（仅 has_git=true 时注入） |

---

## 4. 覆盖优先级（从高到低）

1. `overrideSystemPrompt`（最高）
2. `agentSystemPrompt`
3. `customSystemPrompt`
4. `defaultSystemPrompt`（最低，默认值为 `"You are CloseClaw, a helpful AI assistant."`）
5. `appendSection`（**始终追加**，即使以上都没有也追加）

---

## 5. Append Section 行为

- 最大长度：**500 字符**（`APPEND_SECTION_MAX_LEN`）
- 超出截断，通过 `set_append_section` 返回值（`Option<String>`）通知调用方
- **请求结束后必须调用 `clear_append_section()`**，否则内容泄漏到下一请求
- 渲染格式：`\n\n## Append\n{content}\n`

---

## 6. Channel Context 格式

```markdown
## Channel Context
- chat_name: {chat_name}
- sender_id: {sender_id}
- timestamp: {timestamp}
```

---

## 7. Session State 格式

```markdown
## Session State
- turn_count: {turn_count}
- pending_tasks:
  (none)
  # 或：
  1. task_name
  2. task_name
```

---

## 8. WorkdirContext 数据结构

```rust
pub struct WorkdirContext {
    pub path: String,           // 工作目录绝对路径（canonicalize 后）
    pub has_git: bool,          // 是否为 git repo
    pub branch: Option<String>, // 当前分支（如是 git repo）
    pub recent_changes: usize,  // 未提交变更数（staged + unstaged + untracked）
}
```

### 注入条件
- `set_workdir` 已调用
- `has_git == true`

### gitStatus 渲染格式
```markdown
## Git Status
On branch {branch}
  status: {status_summary}
```
status_summary = `"clean"`（无变更）或 `"{n} uncommitted change(s)"`

---

## 9. 斜杠指令行为

| 指令 | 行为 |
|------|------|
| `/system` | 显示当前追加内容 |
| `/system <text>` | 设置追加内容，超长截断并警告，请求结束后清除 |
| `/cd <path>` | 切换工作目录，返回 WorkdirContext |
| `/pwd` | 返回当前工作目录路径 |
| `/git status` | 返回内嵌 gitStatus（如已 set_workdir 且是 git repo） |
| `/git <args>` | 委托给系统 `git` 二进制执行 |

> ⚠️ `/clear` 命令**未实现**。设计文档中计划用 `/clear` 清除所有静态区缓存，但代码中不存在此指令。

### `/system` 交互文本
- 无内容时：`"当前无追加内容。使用 `/system <内容>` 添加。"`
- 有内容时：显示当前内容 + 用法提示
- 设置后：`"已设置追加内容：\n{content}\n\n请求结束后自动清除。"`
- 截断时：提示替换为 `"已截断至 500 字限制"`

---

## 10. 缓存机制

### mtime 驱动
- `load_cached_file_section(name, path)` 读取文件 mtime，与缓存比较
- mtime 变化 → 缓存失效，重新读取文件

### 核心缓存操作

| 函数 | 签名 | 说明 |
|------|------|------|
| `put_cached_section` | `(name: &str, content: String, file_mtime: Option<u64>)` | 将 section 写入进程级内存缓存（HashMap），`file_mtime` 用于后续 mtime 比对判断缓存新鲜度 |
| `read_file_section` | `(path: &Path) -> Option<(String, u64)>` | 读取文件内容 + mtime，是所有文件驱动 section 的底层 I/O 原语 |
| `load_cached_file_section` | `(name: &str, path: &Path) -> Option<String>` | `read_file_section` + mtime 比较 + `get_cached_section` 命中检测，缓存未命中或 mtime 变化时自动重新加载并写入缓存 |

### 失效触发
1. 文件 mtime 变化（被动的，由 builder 调用时比较）
2. 手动调用 `invalidate_section(name)`
3. 调用 `invalidate_all_sections()`

> ⚠️ **无主动文件监听**。mtime 变化不会主动触发失效，而是在 `build_system_prompt` / `build_from_workspace` 调用时，builder 内部比较当前 mtime 与缓存 mtime 后被动失效。
>
> ⚠️ **`/clear` 命令未实现**。workspace 设计文档中列出 `/clear` 为缓存失效触发条件，但代码中不存在该斜杠指令，失效需通过 `invalidate_all_sections()` 手动调用。

### 缓存键
- 内存 HashMap，键为 section name
- 每个 `CacheEntry { content, file_mtime: Option<u64> }`

---

## 11. 未注入的 Section（见代码）

以下 Section 枚举已定义但**未在 `build_system_prompt` 中注入**：
- `Section::WorkspaceSection` — 预留，未使用
- `Section::ToolsSection` — 预留，未使用

> 如需实现，由调用方在 `build_system_prompt` 前手动构造并传入 sections 参数。

---

## 12. 与权限系统的边界

- `set_workdir` **不进行权限校验**，由调用方确保合法性
- 权限系统以 workdir 为**上下文锚点**：文件操作路径与 workdir 比较判断是否越界
- gitStatus 中的 `recent_changes` 可作为权限系统"可疑操作"信号（变更多的文件风险更高）

---

## 13. 测试覆盖

| 场景 | 测试函数 |
|------|---------|
| mtime 变更触发缓存失效 | `test_cache_stale_on_mtime_change` |
| 手动失效 | `test_invalidate_section` |
| append_section 截断 | `test_append_section_truncation` |
| append_section 清除 | `test_append_section_cleared_after_request` |
| append_section 不为空时不显示 ## Append 标题 | `test_append_section_not_shown_when_empty` |
| 动态 section 不缓存 | `test_dynamic_sections_not_cached` |
| `/system` 空参数显示当前 | `test_system_command_empty_shows_current` |
| `/system` 截断警告 | `test_system_command_truncation` |
| `/pwd` 无 workdir | `test_pwd_command_no_workdir` |
| `/cd` 空参数 | `test_cd_command_empty_args` |
| `/git` 无 workdir | `test_git_command_no_workdir` |

> ⚠️ 以下文档记录的测试**在代码中不存在**：
> - `test_channel_context_freshness`
> - `test_set_workdir_switch`
> - `test_git_status_requires_git`
