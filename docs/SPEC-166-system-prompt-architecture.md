# SPEC-166: System Prompt 分区架构与上下文注入

> 来源：[Issue #166](https://github.com/agent-world/closeclaw/issues/166)
> 状态：`status:confirmed`
> 设计：已接受（issue body 中完整设计）

## 概述

CloseClaw 的 System Prompt 需要一套可维护的分层架构，参考 Claude Code 的设计经验，定义静态区/动态区分区方案。

## 系统架构

### System Prompt 分区

**静态区**（会话级缓存）：

| Section | 来源 | cacheable |
|---------|------|-----------|
| role_section | IDENTITY.md + SOUL.md | true |
| workspace_section | workspace 目录结构 | true |
| tools_section | 所有工具的 prompt | true |

**动态区**（每次请求重建）：

| Section | 来源 | cacheable |
|---------|------|-----------|
| channel_context | 消息上下文（chat_id、chat_type、chat_name、sender_id、timestamp） | false |
| session_state | session 状态（turnCount、pendingTasks） | false |
| memory_section | MEMORY.md | true |
| heartbeat_section | HEARTBEAT.md | true |
| append_section | `/system` 命令追加内容 | false |

**覆盖优先级链**：
1. overrideSystemPrompt（最高）
2. agentSystemPrompt
3. customSystemPrompt
4. defaultSystemPrompt（最低）
5. appendSection（始终追加）

**append_section 规则**：
- 最大长度：500 字，超出截断并提示用户
- 与 AGENTS.md 冲突时：append_section 优先（单次指令覆盖永久定义）
- 缓存失效粒度：section 级

### 飞书上下文注入

- **Session 边界**：飞书话题（thread）= 一个固定话题，不在 prompt 里重复说明
- **每条消息**：带上 sender_id、时间戳、chat_name，作为 message 对象一部分
- **@at 路由**：系统层触发条件，不进 prompt

### gitStatus 接口设计

**核心接口**：
```typescript
type WorkdirContext = {
  path: string           // 工作目录绝对路径
  hasGit: boolean        // 是否为 git repo
  branch?: string        // 当前分支
  recentChanges?: number // 未提交变更数
}
```

**用户斜杠指令**：
- `/cd <path>` — 切换工作目录
- `/pwd` — 查看当前工作目录
- `/git status` — 查看当前目录的 git 状态

**注入条件**：
- 已通过 `set_workdir` 设置工作目录
- 工作目录为 git repo

**workdir 与权限系统的边界**：
- workdir 接口只负责"在哪工作"（上下文锚点、gitStatus 来源）
- 以下文件操作需要权限判断：
  • read/write/exec 等文件操作：权限系统在 workdir 已设置的前提下，根据文件路径判断是否在允许范围内
  • workdir 是权限判断的上下文锚点：设置 workdir 后，后续操作带上该路径前缀，权限系统只判断操作路径是否在 workdir 子树下
  • set_workdir 本身不进行权限校验（权限由调用方确保）
  • 权限系统不感知 gitStatus，但 gitStatus 返回的 recentChanges 可以作为权限系统"可疑操作"的参考信号（变更多的文件风险更高）

## 任务步骤

### Step 1: buildSystemPrompt() 核心函数
- 实现 `buildSystemPrompt()` 核心函数，支持静态区/动态区组装
- 验收：函数接受 sections 列表，返回拼接后的字符串；静态区结果缓存到 session 级别

### Step 2: section 缓存机制
- 实现 section 缓存机制（带失效触发）
- 验收：缓存失效触发条件为：(1) MEMORY.md 变更；(2) heartbeat_section 文件 mtime 变更；(3) 手动调用 invalidateSection(sectionName)。每次 buildSystemPrompt() 前检查 mtime，无变更则复用缓存

### Step 3: append_section（`/system` 命令支持）
- 实现 append_section（`/system` 命令支持）
- 验收：`/system <text>` 命令将 text 追加到 append_section；请求结束后自动清除；长度超过 500 字时截断并提示

### Step 4: channel_context 动态注入
- 实现 channel_context 动态注入（chat_name + sender_id + timestamp）
- 验收：每次 buildSystemPrompt() 都获取最新 chat_name，请求响应中包含该条消息的 sender_id 和 timestamp

### Step 5: set_workdir 接口 + 斜杠指令
- 实现 `set_workdir` 接口 + `/cd` `/pwd` `/git status` 斜杠指令
- 验收：`set_workdir(path: string): { path, hasGit, branch, recentChanges }`，返回工作目录元数据。切换后 gitStatus section 自动注入，gitStatus 只在 hasGit=true 时注入

### Step 6: 单元测试
- 编写单元测试覆盖各 section 缓存失效场景
- 验收：测试覆盖 mtime 变更触发失效、手动 invalidateSection、append_section 请求后清除

**单元测试边界说明**：
- #166：覆盖各 section 缓存失效场景
- #167：覆盖 Schema/Prompt 双层场景（不同测试对象，无冲突）

## 验收标准

- [ ] 静态区在 session 期间不重新构建（除非显式失效）
- [ ] `/system` 命令的追加内容出现在当次请求的 System Prompt 中
- [ ] 请求结束后 append_section 自动清除
- [ ] channel_context 每次请求都反映最新消息上下文（包含 chat_name）
- [ ] `set_workdir` 切换工作目录后，gitStatus section 正确注入
- [ ] section 缓存在文件 mtime 变更时正确失效

## 依赖

- 飞书消息 API（获取群成员、chat_name）
- 工具的 `prompt()` 方法实现（各工具自行定义）
- 权限系统：workdir 是权限判断的上下文锚点
