# 话题与 Session 设计方案

> 状态：已定稿 | 作者：脑暴虾 | 日期：2026-03-24

---

## 文档变更记录

| 日期 | 修改者 | 修改内容 |
|------|--------|----------|
| 2026-03-24 | Braino | 初次整理。识别并修复：① 章节编号残缺（权限章节使用 6.X 编号，与正文 1-5 不衔接）；② TopicSessionMap 文件格式重复出现两次；③ Graceful Shutdown 流程描述重复；④ 配置示例散落在 1.1.1 导致与 1.1 的映射规则描述割裂；⑤ 章节顺序不符合逻辑（实现计划在中间，"其他 IM" 在"风险"之后）。重组后顺序：背景→映射→链接处理→跨 session→生命周期→飞书适配器→其他IM→实现计划→权限→风险。保留所有技术细节不变。 |

---

## 背景与问题

### 现状

CloseClaw 目前的飞书适配器和 Session 管理非常基础：

- **飞书适配器**（`src/im/feishu.rs`）：仅处理文本消息，没有任何 thread/topic 概念，所有消息混在同一个 session 里
- **Gateway Session**（`src/gateway/mod.rs`）：Session 按 `channel:to` 建立，没有 topic 隔离

### 痛点

**核心问题**：飞书天然支持话题（Thread），用户习惯一个话题讨论一件事，但这个能力没有被 CloseClaw 利用。CLI/QQ/微信没有话题机制，需要用指令控制新话题。跨话题上下文无法共享，Session 生命周期没有管理。

### 目标

1. **飞书话题 → CloseClaw Session 1:1 映射**：用户新开话题 = 新建 Session，发消息到已有话题 = 路由到对应 Session
2. **多 IM 统一 Session 抽象**：无论哪个 IM，"话题"概念统一，底层通过 OpenClaw Session 实现
3. **跨 Session 上下文能力**：LLM 可以自主决定是否读取其他 Session 的历史
4. **Session 生命周期管理**：基于活跃时间的分层存储策略

---

## 设计方案

### 1. 飞书话题与 Session 映射

#### 1.1 核心映射规则

| 飞书操作 | CloseClaw 行为 |
|---------|--------------|
| 在话题内发消息 | 路由到该话题对应的 Session |
| 点"新开话题"发消息 | 创建新 Session，绑定该 topic_id |
| 用户@机器人（群聊） | 按 topic_id 路由，无 topic 则走 group Session |
| 私聊 + 回复并开话题 | 用户选择开话题 → 创建新 Session，绑定该 topic_id |
| 私聊 + 普通回复（不开话题） | 走 DM Session（没有 topic） |

**私聊也能开话题**：飞书私聊时，用户可以选择"回复并开新话题"。此时该消息带有 `thread_id`，CloseClaw 按话题路由。只有普通私聊（无 topic）才走 DM Session。

#### 1.2 Session Key 结构

```
# 飞书群聊 + 话题
closeclaw:feishu:group:<chat_id>:topic:<thread_id>

# 飞书私聊
closeclaw:feishu:dm:<open_id>

# CLI（无话题）
closeclaw:cli:dm:<session_id>

# QQ/微信（同 CLI）
closeclaw:qq:dm:<session_id>
closeclaw:wechat:dm:<session_id>
```

#### 1.3 私聊话题配置

私聊消息分为两种场景，需要分别配置：

```rust
pub struct SessionConfig {
    pub fresh_message_topic_mode: TopicCreateMode,
    pub reply_topic_mode: TopicReplyMode,
}

pub enum TopicCreateMode {
    DmSession,     // 走 DM Session（默认）
    CreateTopic,   // Agent 回复时主动创建新 topic
}

pub enum TopicReplyMode {
    FollowUser,    // 用户开了 thread 就跟，用户没开就走 DM（默认）
    AlwaysDm,      // 始终走 DM
    AlwaysThread,  // 回复都开 thread
}
```

**场景一：新消息（不是回复）**

| 模式 | 行为 |
|------|------|
| `dm_session` | 走 DM Session（默认） |
| `create_topic` | Agent 回复时主动创建新 topic 并路由到那里 |

**场景二：回复消息**

| 模式 | 用户开了 thread | 用户没开 thread |
|------|---------------|----------------|
| `follow_user` | 路由到用户的话题 | 走 DM Session（默认） |
| `always_dm` | 路由到用户的话题 | 走 DM Session |
| `always_thread` | 路由到用户的话题 | Agent 回复时主动创建新 topic |

**推荐配置**：`fresh_message_mode = "dm_session"`，`reply_mode = "follow_user"`。这样新对话走 DM 用户无感，有回复行为时自然跟随用户开 topic 的习惯。

**配置示例**：

```json
{
  "session": {
    "private_chat": {
      "fresh_message_mode": "dm_session",
      "reply_mode": "follow_user"
    }
  }
}
```

#### 1.4 飞书事件字段变化

当前 `FeishuEvent` 结构缺少 `thread_id`。飞书 `im.message.receive_v1` 事件中：

```json
{
  "event": {
    "message": {
      "thread_id": "omm_xxx",
      "chat_id": "oc_xxx",
      "message_id": "om_xxx",
      "root_id": "om_yyy"
    }
  }
}
```

**修改点**：`src/im/feishu.rs` 的 `FeishuMessageEvent` 需要增加 `thread_id` 和 `root_id` 字段。

#### 1.5 Topic → Session 绑定表

Gateway 维护一个内存表 + 持久化存储：

```rust
struct TopicSessionMap {
    // key: (chat_id, thread_id) → value: session_id
    mappings: RwLock<HashMap<(String, String), String>>,
}
```

**持久化文件格式**（存储在 `storage_dir / topic_session_map.json`）：

```json
{
  "mappings": {
    "oc_xxx:omm_yyy": "session_abc123",
    "oc_zzz:omm_www": "session_def456"
  },
  "last_updated": 1740441600,
  "version": 1
}
```

**写盘时机**：

| 时机 | 说明 |
|------|------|
| 定期落盘 | 每 N 分钟（或每 N 条消息，可配置） |
| 映射变更后 | 新建 session 或 session 删除时，异步写盘 |
| **关机/重启前** | **Graceful shutdown 时必须落盘** |

**启动恢复**：Gateway 启动时，从 `topic_session_map.json` 加载映射到内存。

**路由逻辑**：

- 有 `thread_id` 的消息：查表 → 命中则路由到对应 session，未命中则创建新 session 并登记映射
- 无 `thread_id` 的消息（DM 或群聊无 topic）：走原有的 DM/group session 逻辑

**Graceful Shutdown 流程**：

```
收到 SIGTERM / SIGINT
    ↓
CloseClaw 执行 graceful shutdown
    ↓
Gateway 收到 shutdown 信号
    ↓
各 IM Adapter 执行 cleanup：
    ├── 飞书适配器：flush TopicSessionMap 到磁盘
    └── 其他 IM Adapter：（各自的 cleanup）
    ↓
Session Store 写盘
    ↓
进程退出
```

```rust
#[async_trait]
pub trait IMAdapter {
    fn name(&self) -> &str;
    async fn handle_webhook(&self, payload: &[u8]) -> Result<Message, AdapterError>;
    async fn send_message(&self, message: &Message) -> Result<(), AdapterError>;
    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool;

    /// 适配器清理，shutdown 前调用
    async fn cleanup(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}
```

如果不等 cleanup 就强制 kill，映射可能丢失。但飞书是 source of truth，按需重建机制可以恢复，只是会导致短暂的路由 miss。

---

### 2. 消息链接处理

#### 2.1 飞书链接格式

| 类型 | 链接格式 |
|------|---------|
| 话题链接 | `https://.feishu.cn/chat/<chat_id>?threadKey=<thread_id>` |
| 消息链接 | `https://.feishu.cn/msg/<message_id>` |

#### 2.2 插件解析行为

当用户发送包含飞书链接的消息时，插件自动解析链接类型和元数据，但**不自动注入上下文**：

1. **正则匹配**：`https://[^\s]+feishu[^\s]+`
2. **解析链接类型**：根据 URL 结构判断是话题还是消息
3. **提取元数据**：话题链接提取 `thread_id`、`chat_id`；消息链接提取 `message_id`
4. **注入给 LLM 的格式**：

```
[来自飞书链接的上下文]
类型：话题/消息
ID：<thread_id 或 message_id>
可用的 Skill：
- feishu_read_thread(thread_id) → 获取话题消息列表
- feishu_read_message(message_id) → 获取单条消息
---
```

#### 2.3 Skill 引导

内置 `feishu-link-navigation` skill，引导 LLM：

- 如果你觉得需要读取 → 直接调用 skill 获取内容
- 如果你不确定是否需要读取 → 询问用户确认
- 发链接通常意味着需要读，不需要读的场景（如仅分享链接）用户会主动说明

#### 2.4 LLM 自主决策

LLM 收到链接元数据后，自行决定：是否读取、读自己还是派 sub-agent 读、读完后如何利用。

#### 2.5 链接指向已删除 Session 的处理（异步重建）

用户发来链接，但对应 session 已被生命周期策略删除：

1. **检测**：查 TopicSessionMap → miss 或 session 已过期
2. **异步重建**：调飞书 API 捞该话题的历史消息，在后台重建 session
3. **用户通知**：立即回复"正在读取，稍等"，不 block LLM
4. **超时机制**：超过 N 秒（如 10s）未完成，告知用户"内容较大，还在处理中"
5. **完成后通知**：重建成功后，主动发消息给用户

**不 block 原则**：整个重建过程在后台进行，不占用 agent 的 LLM 处理流程。

**配置项**：

```json
{
  "session": {
    "rebuild_timeout_seconds": 10,
    "rebuild_notify_interval_seconds": 10
  }
}
```

---

### 3. 跨 Session 上下文

#### 3.1 工具接口

复用 OpenClaw 的 `sessions_history` 和 `sessions_send` 工具，CloseClaw 封装为：

```rust
// Skill: session_read
async fn read_session(session_key: String, limit: usize) -> Vec<Message>

// Skill: session_search
async fn search_sessions(query: String, limit: usize) -> Vec<SessionSummary>
```

#### 3.2 Skill 引导

内置 `session-navigation` skill，引导 LLM 在以下场景使用跨 Session 能力：

- 用户问"之前那个关于 XXX 的话题怎么说的"
- 用户说"去看看oo话题在聊什么"
- 用户发了其他话题的链接

#### 3.3 Sub-agent 总结场景

对于很长的历史 Session，LLM 可以派 sub-agent 去读取并总结，再把结论带回来。减少主 Session 的上下文占用。

---

### 4. Session 生命周期管理

#### 4.1 分层存储策略（基于活跃时间）

**核心原则**：不做硬上限，靠活跃时间自然淘汰。存储够就存，存不下让用户知道。

| 层级 | 格式 | 触发条件 |
|------|------|---------|
| 活跃 | 纯文本 JSON | session 创建后默认状态 |
| 中期 | Binary / ZIP（压缩） | 超过 `storage.medium_threshold` 无活跃（默认 30 天） |
| 长期 | 删除（除非 pinned） | 超过 `storage.long_threshold` 无活跃（默认 90 天） |

以**最后活跃时间**为准。只要有人在话题里发消息，session 就保持活跃状态。

#### 4.2 配置项

```json
{
  "storage": {
    "medium_threshold": "30d",
    "long_threshold": "90d",
    "space_warning_mb": 1024,
    "space_critical_mb": 2048
  }
}
```

`pinned_sessions` 不在全局配置里，Pinned 状态存在每条 session 记录本身，通过 `/pin-session` 指令修改。

#### 4.3 单个 Session 保护

用户可以标记单个 Session 为 `pinned`，则永不被删除（但仍会压缩）。

| 指令 | 说明 |
|------|------|
| `/pin-session` | 固定当前 session |
| `/unpin-session` | 取消固定 |
| `/pin-session <id>` | 固定指定 session（需权限） |

#### 4.4 统计工具

`/storage-stats` 输出活跃时间分布，让用户了解当前分布，自行决定调整阈值。

#### 4.5 空间报警

当存储占用超过阈值时：
- **警告（warning）**：通知用户当前占用，建议检查
- **严重（critical）**：通知用户立即处理，可能触发自动清理

---

### 5. 飞书适配器重构

#### 5.1 事件处理流程

飞书 WebSocket 事件 → 解析 event.content JSON → 检查 thread_id → 查询/创建 Session → 路由到 Gateway

#### 5.2 新增 API 调用

| 功能 | 飞书 API | 用途 |
|------|---------|------|
| 获取话题消息列表 | `GET /im/v1/messages?container_id_type=thread&container_id=<thread_id>` | Session 历史恢复 |
| 获取单条消息 | `GET /im/v1/messages/<message_id>` | 消息链接解析 |
| 发送消息 | `POST /im/v1/messages` | 发送（带 thread_id） |

#### 5.3 错误处理

- **飞书 API 限流**：指数退避重试
- **thread_id 对应的话题已被删除**：回落到 DM session 或提示用户
- **消息链接已失效**：返回"无法获取消息内容"

---

### 6. 其他 IM 的话题支持

| IM | 是否支持话题 | 实现方式 |
|----|------------|---------|
| 飞书 | ✅ 原生支持 | 按 thread_id 路由 |
| Discord | ✅ Forum Channel | 类似飞书，按 thread_id 路由 |
| Telegram | ✅ 话题 | 按 topic_message_thread_id 路由 |
| Slack | ✅ Thread | 按 thread_ts 路由 |
| QQ | ❌ 不支持 | 走 DM Session，无话题隔离 |
| 微信 | ❌ 不支持 | 走 DM Session，无话题隔离 |
| CLI | ❌ 不支持 | 用 `/new` 指令开新 Session |

---

### 7. 实现计划

| Phase | 内容 | 预计工作量 |
|-------|------|----------|
| 1 | 飞书话题映射：修改 `src/im/feishu.rs`，实现 `TopicSessionMap`（含持久化）和路由逻辑 | 中 |
| 2 | 消息链接解析：正则匹配 + 飞书 API 调用 + `feishu-link-navigation` skill | 小 |
| 3 | 跨 Session 工具：`session_read` + `session_search` + `session-navigation` skill | 小 |
| 4 | Session 生命周期：分层存储 + `/pin-session` + `/storage-stats` + 空间报警 | 中 |

---

### 8. 权限设计

#### 8.1 隐私控制：跨 Session 读取权限

当 LLM 想读取其他 Session 时，需要检查请求者是否有权访问。

```
LLM 想读 session_XXX
    ↓
session_XXX 属于 chat_id = "oc_xxx", thread_id = "omm_yyy"
    ↓
当前请求者是谁？→ 从 inbound context 拿到 sender open_id
    ↓
查飞书 API：GET /im/v1/chats/{chat_id}/members
    ├── 用户是成员 → 允许读取
    └── 用户不是成员 → 拒绝，返回"无权访问该话题"
```

**私聊场景**：私聊不存在成员列表问题，Session 权限就是只有 bot 和用户双方，可以直接访问。

#### 8.2 飞书 API 权限检查接口

| 场景 | API | 用途 |
|------|-----|-----|
| 获取群成员列表 | `GET /im/v1/chats/{chat_id}/members` | 检查用户是否是成员 |
| 获取群信息 | `GET /im/v1/chats/{chat_id}` | 检查 bot 是否有权限访问该群 |
| 私聊 | N/A | 私聊 Session 权限仅限 bot + 用户双方 |

#### 8.3 权限引擎集成

上述权限检查由 CloseClaw Permission Engine 执行，Skill 封装为工具：

```rust
// Skill: check_session_access
async fn check_session_access(session_key: String) -> AccessResult
// AccessResult: { allowed: bool, reason: Option<String> }
```

LLM 在读取其他 Session 前，应通过此工具确认有权限。

#### 8.4 TopicSessionMap 不暴露内容

绑定表只存储 `(chat_id, thread_id) → session_id` 的映射关系，不暴露 Session 内容。

#### 8.5 默认行为

- **飞书话题**：受 8.1 权限检查保护
- **私聊 Session**：仅 bot 和用户本人可访问
- **其他 IM**：参照各 IM 的成员/频道机制实现权限检查

---

### 9. 风险与注意事项

1. **飞书 API 限流**：大量消息链接解析可能触发限流，需要退避重试
2. **Session 膨胀**：如果群聊话题极多，靠活跃时间淘汰是主要控制手段
3. **重建延迟**：长期未活跃的 session 被删除后，用户再次访问时需要从飞书 API 重建，有延迟
