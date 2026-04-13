# card 模块规格说明书

## 1. 模块职责

`card` 模块为 CloseClaw 提供飞书富文本交互卡片（interactive card）的数据结构、构建、渲染和事件处理能力，专用于 Plan 模式的进度展示与用户交互。

## 2. 公开接口

### 2.1 导出类型（`mod.rs` re-export）

| 符号 | 来源 | 说明 |
|------|------|------|
| `CardAction` | `elements` | 按钮动作枚举 |
| `CardElement` | `elements` | 卡片元素枚举 |
| `ButtonElement` | `elements` | 按钮元素结构体 |
| `ButtonStyle` | `elements` | 按钮样式枚举 |
| `ImageElement` | `elements` | 图片元素结构体 |
| `MarkdownElement` | `elements` | Markdown 元素结构体 |
| `ProgressElement` | `elements` | 进度条元素结构体 |
| `render_feishu_card` | `renderer` | 将 `RichCard` 渲染为飞书消息 JSON |

### 2.2 模块入口类型

| 类型 | 文件 | 说明 |
|------|------|------|
| `CardHeader` | `mod.rs` | 卡片头部配置 |
| `RichCard` | `mod.rs` | 富文本卡片根结构 |
| `PlanData` | `mod.rs` | Plan 模式数据（输入给 Builder） |
| `PlanStep` | `mod.rs` | 单个步骤 |
| `StepStatus` | `mod.rs` | 步骤状态枚举 |
| `CardEvent` | `events.rs` | 卡片交互产生的事件 |
| `PlanConfirmedPayload` | `events.rs` | PlanConfirmed 事件负载（session_id + card_message_id） |
| `PlanCancelledPayload` | `events.rs` | PlanCancelled 事件负载（session_id） |
| `PlanRegeneratePayload` | `events.rs` | PlanRegenerate 事件负载（session_id） |
| `StepToggledPayload` | `events.rs` | StepToggled 事件负载（step_index + collapsed） |
| `CardError` | `handler.rs` | 处理器错误类型 |
| `CardEventBus` | `handler.rs` | 事件总线 trait |
| `CardUpdateService` | `update.rs` | 卡片更新服务（纯数据构建，无 I/O） |
| `PlanStepUpdate` | `update.rs` | 步骤更新内容 |
| `ProgressUpdate` | `update.rs` | 进度更新内容 |

### 2.3 公开函数

| 函数 | 文件 | 签名 | 说明 |
|------|------|------|------|
| `FeishuCardBuilder::build_plan_card` | `builder.rs` | `(plan: &PlanData) -> RichCard` | 从 PlanData 构建完整 Plan 模式卡片 |
| `CardEvent::from_action` | `events.rs` | `(action: &CardAction, session_id: String, card_message_id: Option<String>) -> Option<Self>` | 将 CardAction 转换为 CardEvent |
| `handle_card_event` | `handler.rs` | `(event: CardEvent, event_bus: &impl CardEventBus) -> Result<(), CardError>` | 处理卡片事件并发布到事件总线 |
| `render_feishu_card` | `renderer.rs` | `(card: &RichCard) -> serde_json::Value` | 渲染完整卡片为飞书 interactive 消息格式 |
| `render_element` | `renderer.rs` | `(element: &CardElement) -> serde_json::Value` | 渲染单个卡片元素 |
| `build_progress_text` | `renderer.rs` | `(current: u32, total: u32, labels: Option<&[String]>) -> String` | 构建进度条文本表示 |
| `CardUpdateService::new` | `update.rs` | `() -> Self` | 构造更新服务 |
| `CardUpdateService::build_step_patch` | `update.rs` | `(step_index: u32, update: &PlanStepUpdate) -> serde_json::Value` | 构建步骤补丁 JSON |
| `CardUpdateService::build_progress_patch` | `update.rs` | `(current: u32, total: u32) -> serde_json::Value` | 构建进度条补丁 JSON |
| `CardUpdateService::build_elements_patch` | `update.rs` | `(elements: Vec<CardElement>) -> serde_json::Value` | 构建完整 elements 数组补丁 |

## 3. 核心数据结构

### 3.1 `CardElement`

卡片元素枚举，JSON 表示为 `{"type": "...", ...}`（`tag = "type"`）。

| variant | 说明 |
|---------|------|
| `Markdown(MarkdownElement)` | Markdown 文本块 |
| `Progress(ProgressElement)` | 进度条（以 Markdown 文本渲染） |
| `Button(ButtonElement)` | 按钮 |
| `Divider` | 分隔线（渲染为 `{"tag": "hr"}`） |
| `Image(ImageElement)` | 图片 |

### 3.2 `CardAction`

按钮动作枚举，JSON 表示为 `{"action": "...", ...}`（`tag = "action"`）。

| variant | 说明 |
|---------|------|
| `ExpandStep { step_index: u32 }` | 展开指定步骤 |
| `CollapseStep { step_index: u32 }` | 折叠指定步骤 |
| `Confirm` | 确认计划 |
| `Cancel` | 取消计划 |
| `Custom { payload: String }` | 自定义动作，payload 为动作标识字符串 |

### 3.3 `ButtonStyle`

| variant | 渲染为飞书 button type |
|---------|----------------------|
| `Primary` | `"primary"` |
| `Secondary` | `"default"` |
| `Default` | `"default"` |

> **注意**：`Secondary` 渲染为 `"default"` 而非 `"secondary"`，因为飞书 button type 只支持 `"primary"` 和 `"default"`。

### 3.4 `StepStatus`

| variant | 状态图标 | 折叠行为 |
|---------|---------|---------|
| `Pending` | `⏳` | 默认折叠（`collapsed = true`） |
| `Active` | `🔄` | 默认展开（`collapsed = false`） |
| `Completed` | `✅` | 默认展开（`collapsed = false`） |

### 3.5 `CardEvent`

| variant | 产生条件 | 发布事件名 |
|---------|---------|-----------|
| `PlanConfirmed { session_id, card_message_id }` | 用户点击 Confirm 按钮 | `"plan_confirmed"` |
| `PlanCancelled { session_id }` | 用户点击 Cancel 按钮 | `"plan_cancelled"` |
| `PlanRegenerate { session_id }` | 用户点击 Regenerate 按钮（payload = "regenerate"） | `"plan_regenerate"` |
| `StepToggled { session_id, step_index, collapsed }` | 用户点击 ExpandStep / CollapseStep 按钮 | `"step_toggled"` |

未知 `Custom` payload 不产生事件（返回 `None`）。

### 3.6 `RichCard`

| 字段 | 类型 | 说明 |
|------|------|------|
| `card_id` | `Option<String>` | 飞书消息 ID，用于后续更新；发送时为 `None` |
| `title` | `String` | 卡片标题 |
| `elements` | `Vec<CardElement>` | 卡片元素有序列表 |
| `header` | `Option<CardHeader>` | 卡片头部；`Some` 时渲染 `header` 节点 |

### 3.7 `PlanData` → `RichCard` 转换规则（`FeishuCardBuilder::build_plan_card`）

1. 第一个元素固定为 `Progress`（当前步骤 / 总步骤 + 步骤标签）
2. 第二个元素固定为 `Divider`
3. 随后按顺序每个步骤一个 `Markdown`，内容格式为 `"状态图标 **标题**\n\n内容"`，可折叠，Pending 状态默认折叠
4. 如果 `is_high_complexity == true`，在步骤后追加 `Divider` + `Button(Confirm, Primary)` + `Button(Regenerate, Secondary)`
5. `header` 固定为 `Some(CardHeader { title: plan.title, subtitle: Some("步骤 x/y"), avatar_url: None })`

## 4. 渲染行为（`renderer.rs`）

### 4.1 `render_feishu_card`

输出 JSON 结构：

```json
{
  "msg_type": "interactive",
  "card": {
    "header": {
      "title": { "tag": "plain_text", "content": "..." },
      "subtitle": { "tag": "plain_text", "content": "..." }
    },
    "elements": [ ... ]
  }
}
```

- `header` 仅在 `card.header.is_some()` 时存在
- `subtitle` 仅在 `header.subtitle.is_some()` 时存在
- `avatar_url` 字段被接受但不渲染（飞书 card header 不支持自定义头像 URL）

### 4.2 进度条文本格式

```
**进度**: ▓▓░░░ 40%
**步骤**: 2/5 — 步骤标签
```

当 `total == 0` 时 percentage 为 0%。步骤标签在 `current <= labels.len()` 时附加到第二行。

### 4.3 按钮渲染格式

```json
{
  "tag": "action",
  "actions": [{
    "tag": "button",
    "text": { "tag": "plain_text", "content": "按钮文字" },
    "type": "primary" | "default"
  }]
}
```

> 飞书 interactive card 的按钮必须包装在 `action` 容器中，且 button 的 `type` 仅支持 `"primary"` 和 `"default"`。

## 5. 卡片更新服务（`update.rs`）

`CardUpdateService` 是一个**纯数据结构**，仅负责构建更新用的 JSON patch，不执行任何 HTTP 请求。

### 5.1 `build_step_patch`

根据 `PlanStepUpdate` 构建 Markdown patch，返回格式：

```json
{
  "tag": "markdown",
  "content": "状态图标 **标题**\n\n内容"
}
```

- `status` 为 `None` 时使用 `"○"` 作为默认图标
- `title` 为 `None` 时使用 `"步骤"`
- `content` 为 `None` 时使用空字符串

### 5.2 `build_progress_patch`

返回格式：

```json
{
  "tag": "markdown",
  "content": "**进度**: ▓▓░░░ 40%\n**步骤**: 2/5"
}
```

不含命名步骤标签（与 `build_progress_text` 不同，后者可传入 `labels` 参数渲染步骤名称）。

### 5.3 `build_elements_patch`

接收 `Vec<CardElement>`，对每个元素调用 `render_element`，返回：

```json
{
  "elements": [ ...渲染后的元素数组... ]
}
```

## 6. 错误类型

### 6.1 `handler.rs` — `CardError`

| variant | 含义 |
|---------|------|
| `EventBus(String)` | 事件总线发布失败 |
| `InvalidAction(String)` | 无效的按钮动作 |

### 6.2 `update.rs` — `CardError`

| variant | 含义 |
|---------|------|
| `CardNotFound(String)` | 卡片不存在（用于未来的 API 集成） |
| `UpdateFailed(String)` | 更新失败（用于未来的 API 集成） |
| `InvalidStepIndex(u32)` | 步骤索引越界 |

## 7. 模块边界

```
card 模块
├── 依赖：serde, serde_json, thiserror, tracing, async_trait
├── 被依赖：im/feishu.rs（用于发送/更新卡片消息）、agent/session 模块（订阅事件）
└── 边界：
    - 本模块不直接发起 HTTP 请求；HTTP 由 FeishuAdapter 执行
    - 本模块不管理 session 状态；session 由 session 模块管理
    - 事件总线由调用方注入（CardEventBus trait），本模块不直接依赖全局 event bus
```

## 8. 与 CARD_INTERACTION_SYSTEM.md 文档的偏差

| # | 偏差描述 | 代码实际情况 | 文档描述 | 影响 |
|---|---------|------------|---------|------|
| 1 | `CardAction` serde tag | `#[serde(tag = "action", rename_all = "snake_case")]` | 文档无 `tag = "action"` | 偏差较小：两者序列化后 JSON 结构一致（`{"action":"confirm"}`） |
| 2 | `ButtonStyle::Secondary` 渲染 | 渲染为 `"default"` | 文档描述为 `"secondary"` | 偏差：Secondary 和 Default 渲染行为相同，均为"default"类型 |
| 3 | `CardEvent::PlanCancelled` 发布事件名 | `"plan_cancelled"` | 文档描述发布 `"mode_switch"` 切换到 direct 模式 | 偏差较大：代码直接发布 `plan_cancelled`，不携带模式切换语义 |
| 4 | `CardUpdateService` | 纯数据构建器，无 I/O | 文档描述持有 `Arc<FeishuAdapter>` 并执行真实 HTTP 更新 | 偏差：update.rs 是纯函数模块，实际 HTTP 调用需在其他模块（如 feishu.rs adapter）实现 |
| 5 | `CardUpdateService` 构造方式 | `CardUpdateService::new()` → `Self` | 文档描述接收 `Arc<FeishuAdapter>` | 无影响：当前设计避免了对 FeishuAdapter 的循环依赖 |
| 6 | `update.rs` 的 `CardError` 定义 | `CardNotFound`, `UpdateFailed`, `InvalidStepIndex` | 文档未定义（作为占位符） | 无影响 |
（已删除 SessionNotFound — 此 variant 不存在于代码中）
| 8 | `CardHeader.avatar_url` | 被存储但 `render_feishu_card` 不渲染 | 文档定义了字段但渲染部分未提及 | 无功能影响：飞书 card header 不支持自定义头像 |
| 9 | `handler.rs` `StepToggled` 日志级别 | `tracing::debug!` | 文档未指定 | 无影响 |
| 10 | `CardEventBus` trait | 存在于 `handler.rs`，被 `handle_card_event` 使用 | 文档描述在 handler.rs 但结构为 async fn | 一致 |
