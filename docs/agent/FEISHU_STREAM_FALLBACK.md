# SPEC: 飞书 Stream→Plan 降级适配

> Issue: [#161](https://github.com/jpxthu/CloseClaw/issues/161)
> 依赖: Issue #160（平台能力检测 + 推理模式基础框架）

## 1. 概述

本文档定义飞书平台的 **Stream→Plan 降级适配**设计。核心目标：

1. 飞书平台不支持流式消息更新（stream 中间输出会被覆盖）
2. 自动将 stream 模式降级为卡片式 Plan 展示
3. 保证用户能看到完整的思考过程
4. 降级延迟 < 500ms（用户无感知）

## 2. 依赖关系

### 2.1 依赖 Issue

- **#160**（平台能力检测 + 推理模式基础框架）提供 `getFallbackMode()` 能力
- Issue #161 的 `FeishuAdapter` 依赖 `PlatformCapabilityService`

### 2.2 被依赖 Issue

- **#163**（卡片交互系统）依赖本 Issue 定义的卡片结构

## 3. 核心设计

### 3.1 降级触发流程

```
用户触发 stream 模式
  ↓
FeishuAdapter.shouldFallback(stream) → true
  ↓
PlatformCapabilityService.getFallbackMode("feishu", "stream") → "plan"
  ↓
executeFallback() → 发送初始提示 → 创建 Plan 卡片 → 逐步更新卡片 → 发送最终结论
```

### 3.2 FeishuAdapter 核心实现

```rust
// src/platform/feishu/adapter.rs

use crate::platform::{PlatformAdapter, PlatformCapability, ReasoningMode};

/// 飞书平台适配器
pub struct FeishuAdapter {
    capability_service: Arc<PlatformCapabilityService>,
    card_service: Arc<dyn CardService>,
    message_service: Arc<dyn FeishuMessageService>,
}

impl FeishuAdapter {
    /// 判断是否需要降级
    pub fn should_fallback(&self, mode: ReasoningMode) -> bool {
        mode == ReasoningMode::Stream
            && !self.capability_service.supports_mode("feishu", ReasoningMode::Stream)
    }

    /// 执行降级展示流程
    pub async fn execute_fallback(
        &self,
        intent: &ModeSwitchEvent,
    ) -> Result<FallbackResult, FeishuAdapterError> {
        // Step 1: 发送初始提示消息
        let initial = self.send_initial_message().await?;

        // Step 2: 创建 Plan 卡片框架
        let card = self.create_plan_card(intent.user_intent.as_ref()).await?;

        // Step 3: 逐步更新卡片内容
        self.run_streaming_with_card_update(&card, intent).await?;

        // Step 4: 发送最终结论
        let final_msg = self.send_final_message(&card).await?;

        Ok(FallbackResult {
            initial_message_id: initial,
            card_message_id: card.id,
            final_message_id: final_msg,
        })
    }
}
```

### 3.3 降级步骤定义

```rust
// src/platform/feishu/fallback.rs

/// 飞书降级执行步骤
#[derive(Debug, Clone)]
pub struct FallbackStep {
    pub step: u32,
    pub action: FallbackAction,
    pub content: String,
    pub persist: bool,
}

/// 降级动作类型
#[derive(Debug, Clone)]
pub enum FallbackAction {
    /// 发送初始提示
    SendInitialMessage,
    /// 创建交互卡片
    CreateCard,
    /// 更新卡片内容
    UpdateCard,
    /// 发送最终结论
    SendFinal,
}

/// 默认降级步骤
pub const FEISHU_STREAM_FALLBACK_STEPS: &[FallbackStep] = &[
    FallbackStep {
        step: 1,
        action: FallbackAction::SendInitialMessage,
        content: "🔍 进入深度分析模式...".to_string(),
        persist: true,
    },
    FallbackStep {
        step: 2,
        action: FallbackAction::CreateCard,
        content: "创建 Plan 卡片框架".to_string(),
        persist: false,
    },
    FallbackStep {
        step: 3,
        action: FallbackAction::UpdateCard,
        content: "逐步更新卡片内容".to_string(),
        persist: false,
    },
    FallbackStep {
        step: 4,
        action: FallbackAction::SendFinal,
        content: "发送最终结论".to_string(),
        persist: true,
    },
];
```

### 3.4 Plan 卡片初始结构

```rust
// src/platform/feishu/card.rs

use serde::{Deserialize, Serialize};

/// Plan 卡片配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCardConfig {
    pub title: String,
    pub sections: Vec<PlanSection>,
    pub show_progress: bool,
    pub show_step_buttons: bool,
}

/// Plan 卡片中的单个步骤
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSection {
    pub step_number: u32,
    pub title: String,
    pub content: String,
    pub status: StepStatus,
}

/// 步骤状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Pending,
    Active,
    Completed,
}

/// 构建初始卡片 sections
pub fn build_initial_sections(goal: Option<&str>) -> Vec<PlanSection> {
    vec![
        PlanSection {
            step_number: 1,
            title: "需求分析".to_string(),
            content: goal.map(|g| format!("目标：{}", g)).unwrap_or_else(|| "分析中...".to_string()),
            status: StepStatus::Pending,
        },
        PlanSection {
            step_number: 2,
            title: "技术方案".to_string(),
            content: "待确定...".to_string(),
            status: StepStatus::Pending,
        },
        PlanSection {
            step_number: 3,
            title: "实现路径".to_string(),
            content: "待确定...".to_string(),
            status: StepStatus::Pending,
        },
    ]
}
```

### 3.5 卡片更新服务

```rust
// src/platform/feishu/card_updater.rs

use async_trait::async_trait;

/// 卡片更新服务接口
#[async_trait]
pub trait CardService: Send + Sync {
    /// 创建交互卡片
    async fn create_card(
        &self,
        config: &PlanCardConfig,
    ) -> Result<CardHandle, CardServiceError>;

    /// 更新卡片中的某个 section
    async fn update_section(
        &self,
        card_id: &str,
        section_index: usize,
        update: SectionUpdate,
    ) -> Result<(), CardServiceError>;

    /// 更新卡片进度
    async fn update_progress(
        &self,
        card_id: &str,
        current_step: u32,
        total_steps: u32,
    ) -> Result<(), CardServiceError>;

    /// 标记步骤完成
    async fn mark_step_complete(
        &self,
        card_id: &str,
        step_number: u32,
    ) -> Result<(), CardServiceError>;
}

/// Section 更新内容
#[derive(Debug, Clone)]
pub struct SectionUpdate {
    pub title: Option<String>,
    pub content: Option<String>,
    pub status: Option<StepStatus>,
}

/// 卡片句柄
#[derive(Debug, Clone)]
pub struct CardHandle {
    pub message_id: String,
}
```

### 3.6 逐步更新卡片

```rust
// src/platform/feishu/updater.rs

impl FeishuAdapter {
    /// 逐步更新卡片（模拟 stream 效果）
    async fn run_streaming_with_card_update(
        &self,
        card: &CardHandle,
        intent: &ModeSwitchEvent,
    ) -> Result<(), FeishuAdapterError> {
        // LLM 流式输出时，将每个完整句子更新到对应 section
        // 每次 LLM 输出完整句子后：
        // - 找到当前活跃的 section
        // - 更新其 content
        // - 调用 card_service.update_section() 刷新卡片

        let sections = build_initial_sections(intent.user_intent.as_ref().map(|u| u.parsed_goal.as_str()));

        // 更新进度为第一步
        self.card_service.update_progress(&card.message_id, 1, sections.len() as u32).await?;

        for (idx, section) in sections.iter().enumerate() {
            // 标记当前 step 为 active
            let mut active_update = SectionUpdate::default();
            active_update.status = Some(StepStatus::Active);
            self.card_service.update_section(&card.message_id, idx, active_update).await?;

            // 模拟 LLM 输出更新 content（实际由 LLM 服务调用）
            // 这里由调用方通过 update_card_content() 触发

            // 标记为 completed
            self.card_service.mark_step_complete(&card.message_id, section.step_number).await?;
        }

        Ok(())
    }
}
```

### 3.7 高复杂度任务判断

```rust
// src/platform/feishu/complexity.rs

/// 高复杂度判断标准
pub fn is_high_complexity(intent: &ModeSwitchEvent) -> bool {
    let goal = intent.user_intent.as_ref()
        .and_then(|u| u.parsed_goal.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("");

    let complexity_indicators = [
        "系统", "架构", "设计", "实现", "重构", "迁移"
    ];

    let indicator_count = complexity_indicators.iter()
        .filter(|k| goal.contains(*k))
        .count();

    indicator_count >= 2 || goal.len() > 100
}

/// 高复杂度任务的增强配置
#[derive(Debug, Clone)]
pub struct HighComplexityConfig {
    pub show_progress_bar: bool,
    pub show_key_decision_points: bool,
    pub enable_mind_map_export: bool,
    pub enable_step_confirmation: bool,
}

impl Default for HighComplexityConfig {
    fn default() -> Self {
        Self {
            show_progress_bar: true,
            show_key_decision_points: true,
            enable_mind_map_export: false,
            enable_step_confirmation: true,
        }
    }
}
```

## 4. 配置

### 4.1 配置项

```yaml
# config/default.yaml

feishu:
  stream_fallback:
    # 是否启用 stream 降级
    enabled: true
    # 降级后的初始消息
    initial_message: "🔍 进入深度分析模式..."
    # 降级延迟阈值（毫秒），超过则强制降级
    fallback_delay_threshold_ms: 500
```

## 5. 错误处理

```rust
// src/platform/feishu/error.rs

use thiserror::Error;

#[derive(Error, Debug)]
pub enum FeishuAdapterError {
    #[error("Card service error: {0}")]
    CardService(String),

    #[error("Message service error: {0}")]
    MessageService(String),

    #[error("Capability service error: {0}")]
    CapabilityService(String),

    #[error("Fallback not enabled")]
    FallbackNotEnabled,

    #[error("Card not found: {0}")]
    CardNotFound(String),
}
```

## 6. 验收标准

- [ ] 飞书适配器正确识别 `stream` 模式并触发降级
- [ ] 降级后的 Plan 卡片包含完整的思考框架 sections
- [ ] 每个 LLM 输出句子能正确更新到对应 section（卡片实时刷新）
- [ ] 进度条正确显示当前步骤和总步骤数
- [ ] 降级延迟 < 500ms（用户无感知）
- [ ] 网关重启后，降级中的卡片能正确恢复显示（依赖 #159）
- [ ] 降级流程可通过配置开关（`feishu.stream_fallback.enabled: true/false`）
- [ ] 高复杂度任务（> 3 steps）显示操作按钮

## 7. 文件结构

```
src/platform/
├── mod.rs                    # Platform 模块入口
├── capability.rs              # 平台能力检测（来自 #160）
└── feishu/
    ├── mod.rs               # 飞书适配器模块入口
    ├── adapter.rs           # FeishuAdapter 核心实现
    ├── fallback.rs          # 降级步骤定义
    ├── card.rs              # Plan 卡片结构
    ├── card_updater.rs      # 卡片更新服务接口
    ├── updater.rs           # 逐步更新逻辑
    ├── complexity.rs         # 高复杂度判断
    └── error.rs             # 错误类型

docs/agent/
    └── FEISHU_STREAM_FALLBACK.md   # 本文档
```
