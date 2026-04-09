# SPEC: 卡片交互系统 — 富文本卡片 + 进度展示

> Issue: [#163](https://github.com/jpxthu/CloseClaw/issues/163)

## 1. 概述

本文档定义 CloseClaw 的**飞书卡片交互系统**。核心目标：

1. 为 Plan 模式提供富文本卡片展示（进度条、步骤列表、操作按钮）
2. 支持卡片的增量更新（模拟思考过程逐步展示）
3. 支持按钮交互处理（确认计划、重新调整）
4. 依赖 #159（Session 持久化）和 #161（飞书降级适配）

## 2. 现有结构

### 2.1 飞书适配器

`src/im/feishu.rs` 中已有 `FeishuAdapter`，负责消息发送和接收。

### 2.2 消息类型

当前支持 `text` 消息类型，需要扩展 `interactive`（卡片）消息类型。

## 3. 数据结构设计

### 3.1 卡片元素类型

```rust
// src/card/elements.rs

use serde::{Deserialize, Serialize};

/// 卡片元素枚举
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CardElement {
    /// Markdown 文本块
    Markdown(MarkdownElement),
    /// 进度条
    Progress(ProgressElement),
    /// 按钮
    Button(ButtonElement),
    /// 分隔线
    Divider,
    /// 图片
    Image(ImageElement),
}

/// Markdown 文本块
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkdownElement {
    pub content: String,           // 支持的 markdown：bold, italic, code, list, link
    pub collapsible: bool,        // 是否可折叠
    pub collapsed: bool,          // 默认折叠状态
}

/// 进度条
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressElement {
    pub current: u32,             // 当前步骤（1-indexed）
    pub total: u32,                // 总步骤数
    pub labels: Option<Vec<String>>, // 每一步的标签
}

/// 按钮
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ButtonElement {
    pub text: String,
    pub action: CardAction,
    pub style: ButtonStyle,
}

/// 按钮样式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Default,
}

/// 卡片动作
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardAction {
    /// 展开步骤
    ExpandStep { step_index: u32 },
    /// 折叠步骤
    CollapseStep { step_index: u32 },
    /// 确认计划
    Confirm,
    /// 取消计划
    Cancel,
    /// 自定义动作
    Custom { payload: String },
}

/// 图片元素
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageElement {
    pub url: String,
    pub alt: Option<String>,
}
```

### 3.2 富文本卡片结构

```rust
// src/card/mod.rs

use super::elements::CardElement;
use serde::{Deserialize, Serialize};

/// 富文本卡片
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichCard {
    /// 飞书消息 ID（用于更新）
    pub card_id: Option<String>,
    /// 卡片标题
    pub title: String,
    /// 卡片元素列表
    pub elements: Vec<CardElement>,
    /// 卡片头部
    pub header: Option<CardHeader>,
}

/// 卡片头部
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardHeader {
    pub title: String,
    pub subtitle: Option<String>,
    pub avatar_url: Option<String>,
}

/// Plan 数据（用于构建 Plan 模式卡片）
#[derive(Debug, Clone)]
pub struct PlanData {
    pub title: String,
    pub current_step: u32,
    pub total_steps: u32,
    pub step_labels: Vec<String>,
    pub steps: Vec<PlanStep>,
    pub is_high_complexity: bool,
}

/// Plan 步骤
#[derive(Debug, Clone)]
pub struct PlanStep {
    pub title: String,
    pub content: String,
    pub status: StepStatus,
}

/// 步骤状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Active,
    Completed,
}
```

## 4. 飞书卡片构建器

### 4.1 FeishuCardBuilder

```rust
// src/card/builder.rs

use super::{CardElement, PlanData, PlanStep, RichCard, StepStatus};
use super::elements::{ButtonElement, ButtonStyle, CardAction, MarkdownElement, ProgressElement};
use crate::card::renderer::render_element;

/// 飞书卡片构建器
pub struct FeishuCardBuilder;

impl FeishuCardBuilder {
    /// 构建 Plan 模式卡片
    pub fn build_plan_card(plan: &PlanData) -> RichCard {
        let mut elements = Vec::new();

        // 进度条
        elements.push(CardElement::Progress(ProgressElement {
            current: plan.current_step,
            total: plan.total_steps,
            labels: Some(plan.step_labels.clone()),
        }));

        // 分隔线
        elements.push(CardElement::Divider);

        // 各步骤内容
        for step in &plan.steps {
            elements.push(CardElement::Markdown(Self::format_step_content(step)));
        }

        // 操作按钮（高复杂度任务显示）
        if plan.is_high_complexity {
            elements.push(CardElement::Button(ButtonElement {
                text: "✅ 确认计划".to_string(),
                action: CardAction::Confirm,
                style: ButtonStyle::Primary,
            }));
            elements.push(CardElement::Button(ButtonElement {
                text: "🔄 重新调整".to_string(),
                action: CardAction::Custom { payload: "regenerate".to_string() },
                style: ButtonStyle::Secondary,
            }));
        }

        RichCard {
            card_id: None,
            title: plan.title.clone(),
            elements,
            header: Some(CardHeader {
                title: plan.title.clone(),
                subtitle: Some(format!("步骤 {}/{}", plan.current_step, plan.total_steps)),
                avatar_url: None,
            }),
        }
    }

    /// 格式化步骤内容
    fn format_step_content(step: &PlanStep) -> CardElement {
        let status_icon = match step.status {
            StepStatus::Pending => "⏳",
            StepStatus::Active => "🔄",
            StepStatus::Completed => "✅",
        };

        let content = format!(
            "{} **{}**\n\n{}",
            status_icon,
            step.title,
            step.content
        );

        CardElement::Markdown(MarkdownElement {
            content,
            collapsible: true,
            collapsed: step.status == StepStatus::Pending,
        })
    }
}
```

## 5. 卡片渲染

### 5.1 渲染为飞书 interactive card 格式

```rust
// src/card/renderer.rs

use super::elements::{ButtonElement, ButtonStyle, CardAction, CardElement, ImageElement, MarkdownElement, ProgressElement};
use serde::Serialize;

/// 渲染元素为飞书格式
pub fn render_element(element: &CardElement) -> serde_json::Value {
    match element {
        CardElement::Markdown(el) => render_markdown(el),
        CardElement::Progress(el) => render_progress(el),
        CardElement::Button(el) => render_button(el),
        CardElement::Divider => serde_json::json!({ "tag": "hr" }),
        CardElement::Image(el) => render_image(el),
    }
}

/// 渲染 Markdown 元素
fn render_markdown(el: &MarkdownElement) -> serde_json::Value {
    serde_json::json!({
        "tag": "markdown",
        "content": el.content
    })
}

/// 渲染进度条
fn render_progress(el: &ProgressElement) -> serde_json::Value {
    serde_json::json!({
        "tag": "markdown",
        "content": build_progress_text(el.current, el.total, el.labels.as_deref())
    })
}

/// 构建进度条文本
fn build_progress_text(current: u32, total: u32, labels: Option<&[String]>) -> String {
    let filled = "▓".repeat(current as usize);
    let empty = "░".repeat((total - current) as usize);
    let percentage = if total > 0 {
        (current * 100 / total) as u32
    } else {
        0
    };

    let mut text = format!("**进度**: {}{} {}%\n", filled, empty, percentage);
    text.push_str(&format!("**步骤**: {}/{}", current, total));

    if let Some(labels) = labels {
        if !labels.is_empty() && (current as usize) < labels.len() {
            text.push_str(&format!(" — {}", labels[(current - 1) as usize]));
        }
    }

    text
}

/// 渲染按钮
fn render_button(el: &ButtonElement) -> serde_json::Value {
    let button_type = match el.style {
        ButtonStyle::Primary => "primary",
        _ => "default",
    };

    serde_json::json!({
        "tag": "action",
        "actions": [{
            "tag": "button",
            "text": { "tag": "plain_text", "content": el.text },
            "type": button_type
        }]
    })
}

/// 渲染图片
fn render_image(el: &ImageElement) -> serde_json::Value {
    serde_json::json!({
        "tag": "img",
        "img_key": el.url,  // 飞书需要先上传图片获取 img_key
        "alt": { "tag": "plain_text", "content": el.alt.as_deref().unwrap_or("") }
    })
}

/// 渲染完整卡片为飞书消息格式
pub fn render_feishu_card(card: &RichCard) -> serde_json::Value {
    let mut card_json = serde_json::json!({
        "msg_type": "interactive",
        "card": {
            "elements": card.elements.iter().map(render_element).collect::<Vec<_>>()
        }
    });

    // 添加 header
    if let Some(ref header) = card.header {
        let header_json = serde_json::json!({
            "header": {
                "title": {
                    "tag": "plain_text",
                    "content": header.title
                },
                "subtitle": header.subtitle.as_ref().map(|s| {
                    serde_json::json!({ "tag": "plain_text", "content": s })
                })
            }
        });
        if let Some(obj) = card_json.as_object_mut() {
            if let Some(card_obj) = obj.get_mut("card").and_then(|c| c.as_object_mut()) {
                card_obj.insert("header".to_string(), header_json["header"].clone());
            }
        }
    }

    card_json
}
```

## 6. 卡片更新服务

### 6.1 CardUpdateService

```rust
// src/card/update.rs

use crate::im::feishu::FeishuAdapter;
use std::sync::Arc;

/// 卡片更新服务
pub struct CardUpdateService {
    feishu_adapter: Arc<FeishuAdapter>,
}

impl CardUpdateService {
    pub fn new(feishu_adapter: Arc<FeishuAdapter>) -> Self {
        Self { feishu_adapter }
    }

    /// 增量更新某个 step 的内容
    pub async fn update_step(
        &self,
        card_message_id: &str,
        step_index: u32,
        update: &PlanStepUpdate,
    ) -> Result<(), CardError> {
        let patch_content = self.build_step_patch(step_index, update);
        self.feishu_adapter.update_message(card_message_id, &patch_content).await
    }

    /// 刷新进度条
    pub async fn update_progress(
        &self,
        card_message_id: &str,
        current: u32,
        total: u32,
    ) -> Result<(), CardError> {
        let progress_element = serde_json::json!({
            "tag": "markdown",
            "content": super::renderer::build_progress_text(current, total, None)
        });

        // 飞书卡片更新：替换第一个元素（进度条）
        let patch = serde_json::json!({
            "elements": serde_json::json!([progress_element, { "tag": "hr" }])
        });

        self.feishu_adapter.update_message(card_message_id, &patch).await
    }
}

/// 步骤更新内容
#[derive(Debug, Clone)]
pub struct PlanStepUpdate {
    pub title: Option<String>,
    pub content: Option<String>,
    pub status: Option<StepStatus>,
}
```

## 7. 按钮交互处理

### 7.1 事件定义

```rust
// src/card/events.rs

use super::elements::CardAction;

/// 卡片按钮事件
#[derive(Debug, Clone)]
pub enum CardEvent {
    /// 用户确认计划
    PlanConfirmed { session_id: String, card_message_id: String },
    /// 用户取消计划
    PlanCancelled { session_id: String },
    /// 重新生成计划
    PlanRegenerate { session_id: String },
    /// 展开/折叠步骤
    StepToggled { session_id: String, step_index: u32, collapsed: bool },
}

impl CardEvent {
    pub fn from_action(
        action: &CardAction,
        session_id: String,
        card_message_id: Option<String>,
    ) -> Option<Self> {
        match action {
            CardAction::Confirm => Some(CardEvent::PlanConfirmed {
                session_id,
                card_message_id: card_message_id.unwrap_or_default(),
            }),
            CardAction::Cancel => Some(CardEvent::PlanCancelled { session_id }),
            CardAction::Custom { payload } if payload == "regenerate" => {
                Some(CardEvent::PlanRegenerate { session_id })
            }
            CardAction::ExpandStep { step_index } => Some(CardEvent::StepToggled {
                session_id,
                step_index: *step_index,
                collapsed: false,
            }),
            CardAction::CollapseStep { step_index } => Some(CardEvent::StepToggled {
                session_id,
                step_index: *step_index,
                collapsed: true,
            }),
            _ => None,
        }
    }
}
```

### 7.2 按钮点击处理

```rust
// src/card/handler.rs

use super::events::CardEvent;

/// 处理卡片按钮点击
pub async fn handle_card_action(
    event: CardEvent,
    event_bus: &Arc<dyn EventBus>,
) -> Result<(), CardError> {
    match event {
        CardEvent::PlanConfirmed { session_id, card_message_id } => {
            tracing::info!(session_id = %session_id, card_message_id = %card_message_id, "plan confirmed");
            event_bus.publish("plan_confirmed", PlanConfirmedPayload { session_id, card_message_id }).await?;
        }
        CardEvent::PlanCancelled { session_id } => {
            tracing::info!(session_id = %session_id, "plan cancelled");
            event_bus.publish("mode_switch", ModeSwitchPayload {
                session_id,
                from_mode: "plan",
                to_mode: "direct",
                trigger: "user_cancelled".to_string(),
            }).await?;
        }
        CardEvent::PlanRegenerate { session_id } => {
            tracing::info!(session_id = %session_id, "plan regenerate requested");
            event_bus.publish("plan_regenerate", PlanRegeneratePayload { session_id }).await?;
        }
        CardEvent::StepToggled { session_id, step_index, collapsed } => {
            tracing::debug!(session_id = %session_id, step_index = %step_index, collapsed = %collapsed, "step toggled");
            // 通知 UI 层更新折叠状态
            event_bus.publish("step_toggled", StepToggledPayload { session_id, step_index, collapsed }).await?;
        }
    }
    Ok(())
}
```

## 8. 集成到飞书适配器

### 8.1 发送卡片消息

```rust
// src/im/feishu.rs

impl FeishuAdapter {
    /// 发送卡片消息
    pub async fn send_card(
        &self,
        chat_id: &str,
        card: &RichCard,
    ) -> Result<String, AdapterError> {
        let payload = render_feishu_card(card);
        let message_id = self.send_message(chat_id, &payload).await?;
        Ok(message_id)
    }

    /// 更新卡片消息
    pub async fn update_message(
        &self,
        message_id: &str,
        patch: &serde_json::Value,
    ) -> Result<(), AdapterError> {
        // 飞书不支持部分更新，需要替换整个卡片内容
        // 通过 patch 传入新的 elements 数组
        let url = format!("{}/im/v1/messages/{}", FEISHU_API_BASE, message_id);
        let body = serde_json::json!({
            "content": serde_json::to_string(patch).unwrap()
        });

        // ... HTTP PATCH request
        Ok(())
    }
}
```

## 9. 配置

### 9.1 配置项

```yaml
# config/default.yaml

card:
  # 卡片更新超时（毫秒）
  update_timeout_ms: 500

  # 高复杂度阈值（步骤数 > 此值显示按钮）
  high_complexity_threshold: 3

  # 是否启用卡片交互
  enabled: true
```

## 10. 文件结构

```
src/card/
├── mod.rs                     # 模块入口，导出主要类型
├── elements.rs                # 卡片元素数据结构
├── builder.rs                 # FeishuCardBuilder
├── renderer.rs                # 飞书卡片渲染器
├── update.rs                  # CardUpdateService
├── events.rs                 # 卡片事件定义
└── handler.rs                # 按钮交互处理

src/im/
└── feishu.rs                 # 扩展 FeishuAdapter 支持卡片消息
```

## 11. 验收标准

- [ ] `RichCard` 和 `CardElement` 数据结构完整定义
- [ ] `FeishuCardBuilder::build_plan_card()` 生成的卡片包含进度条、步骤列表、按钮
- [ ] 进度条正确显示当前步骤和总步骤数（如 "▓▓▓░░ 60%"）
- [ ] 步骤内容支持 markdown 格式（bold/code/list）
- [ ] 点击"确认计划"按钮触发 `plan_confirmed` 事件
- [ ] 点击"重新调整"按钮触发 `plan_regenerate` 事件
- [ ] 卡片更新（progress 刷新、step 内容更新）支持
- [ ] 卡片消息 ID 被记录，用于后续更新
- [ ] 高复杂度任务（> 3 steps）的卡片显示操作按钮
- [ ] 单元测试覆盖核心逻辑
