# SPEC: System Prompt 分区架构与上下文注入

> Issue: #166 — System Prompt 分区架构与上下文注入

## 1. 概述

实现一套分层的 System Prompt 构建系统，支持静态区缓存和动态区实时注入，提高上下文构建的可维护性和性能。

## 2. 核心模块

新建 `src/system_prompt/` 模块，包含以下文件：
- `mod.rs` — 导出公共类型
- `sections.rs` — Section 定义与缓存逻辑
- `builder.rs` — `buildSystemPrompt()` 核心函数
- `workdir.rs` — 工作目录上下文与 gitStatus
- `slash_commands.rs` — `/system`、`/cd`、`/pwd`、`/git` 斜杠指令

## 3. Section 定义

### 3.1 静态区（session 级缓存）

| Section | 来源 | cacheable |
|---------|------|-----------|
| `role_section` | IDENTITY.md + SOUL.md | true |
| `workspace_section` | workspace 目录结构 | true |
| `tools_section` | 所有工具的 `prompt()` 方法 | true |
| `memory_section` | MEMORY.md | true |
| `heartbeat_section` | HEARTBEAT.md | true |

### 3.2 动态区（每次请求重建）

| Section | 来源 | cacheable |
|---------|------|-----------|
| `channel_context` | chat_name、sender_id、timestamp | false |
| `session_state` | turnCount、pendingTasks | false |
| `append_section` | `/system` 命令追加内容 | false |
| `git_status` | 工作目录 git 状态 | false |

## 4. Section 缓存机制

### 4.1 缓存键
- `section:<name>:<file_mtime>` — 文件 section 使用文件 mtime 作为缓存键

### 4.2 缓存失效触发条件
1. MEMORY.md 文件 mtime 变更
2. heartbeat_section 文件（HEARTBEAT.md）mtime 变更
3. 手动调用 `invalidate_section(section_name)`

### 4.3 `buildSystemPrompt()` 行为
1. 检查每个静态 section 的缓存有效性（mtime）
2. 无变更 → 返回缓存内容
3. 有变更 → 重新构建并更新缓存
4. 所有动态 section 每次都重新构建

## 5. Append Section（`/system` 命令）

### 5.1 行为
- `/system <text>` 将 text 追加到 `append_section`
- append_section 最大长度：**500 字**，超出截断并提示用户
- append_section 在**请求结束后自动清除**
- 与 AGENTS.md 冲突时：append_section 优先（单次指令覆盖永久定义）

### 5.2 实现
- `append_section: Option<String>` 字段存在于请求级上下文
- 不持久化，请求结束后立即清除

## 6. Channel Context 注入

每次 `buildSystemPrompt()` 调用时注入：
- `chat_name` — 来自飞书消息事件
- `sender_id` — 来自飞书消息事件
- `timestamp` — 消息时间戳

格式：
```
## Channel Context
- chat_name: xxx
- sender_id: ou_xxx
- timestamp: 2026-04-10T15:32:00+08:00
```

## 7. Workdir Context 与 gitStatus

### 7.1 `set_workdir(path: String) -> WorkdirContext`

```rust
pub struct WorkdirContext {
    pub path: String,           // 工作目录绝对路径
    pub has_git: bool,          // 是否为 git repo
    pub branch: Option<String>,  // 当前分支
    pub recent_changes: usize,   // 未提交变更数
}
```

### 7.2 斜杠指令

| 指令 | 行为 |
|------|------|
| `/cd <path>` | 调用 `set_workdir`，返回新目录元数据 |
| `/pwd` | 返回当前工作目录 |
| `/git status` | 返回 gitStatus（仅 has_git=true 时） |

### 7.3 gitStatus Section 注入条件
- 已调用 `set_workdir`
- `has_git == true`

### 7.4 与权限系统的边界
- `set_workdir` 不进行权限校验（调用方确保合法性）
- 权限系统以 workdir 为上下文锚点判断文件操作是否在允许范围内

## 8. 覆盖优先级

1. `overrideSystemPrompt`（最高）
2. `agentSystemPrompt`
3. `customSystemPrompt`
4. `defaultSystemPrompt`（最低）
5. `append_section`（始终追加）

## 9. 公开 API

```rust
// 核心构建函数
pub fn build_system_prompt(sections: Vec<Section>) -> String

// Section 缓存管理
pub fn invalidate_section(name: &str)
pub fn get_cached_section(name: &str) -> Option<String>

// Append section（请求级）
pub fn set_append_section(text: String) -> String  // 超过500字时返回截断提示
pub fn clear_append_section()

// Workdir
pub fn set_workdir(path: String) -> WorkdirContext
pub fn get_workdir() -> Option<String>

// Slash commands
pub fn handle_system_command(args: &str) -> SlashCommandResult
pub fn handle_cd_command(args: &str) -> SlashCommandResult
pub fn handle_pwd_command() -> SlashCommandResult
pub fn handle_git_command(args: &str) -> SlashCommandResult
```

## 10. 单元测试覆盖

| 测试场景 | 覆盖 |
|---------|------|
| mtime 变更触发缓存失效 | `test_cache_invalidates_on_mtime_change` |
| 手动 `invalidate_section` | `test_manual_invalidate_section` |
| append_section 请求后清除 | `test_append_section_cleared_after_request` |
| append_section 超长截断 | `test_append_section_truncation` |
| channel_context 每次最新 | `test_channel_context_freshness` |
| `set_workdir` 切换目录 | `test_set_workdir_switch` |
| gitStatus 仅在 has_git 时注入 | `test_git_status_requires_git` |

## 11. 验收标准

- [ ] 静态区在 session 期间不重新构建（除非显式失效）
- [ ] `/system` 命令的追加内容出现在当次请求的 System Prompt 中
- [ ] 请求结束后 append_section 自动清除
- [ ] channel_context 每次请求都反映最新消息上下文（包含 chat_name）
- [ ] `set_workdir` 切换工作目录后，gitStatus section 正确注入
- [ ] section 缓存在文件 mtime 变更时正确失效
