# Platform 模块规格说明书

> 本文件描述模块「当前是什么」，不包含开发步骤、issue 号或验收标准。

---

## 1. 模块概述

**职责**：提供多平台能力检测与推理模式降级支持，是下游功能的基础设施。

**下游依赖**：
- Issue #161：Feishu Stream→Plan 降级
- Issue #162：斜杠指令系统
- Issue #163：卡片交互系统

**子模块**：
- `src/platform/` — 平台能力检测核心
- `src/platform/feishu/` — 飞书平台专用适配器（Stream→Plan 降级）

---

## 2. 公开 API

### 2.1 platform/mod.rs — 模块入口

```rust
// 常量
pub const PLATFORM_FEISHU: &str = "feishu";
pub const PLATFORM_TELEGRAM: &str = "telegram";
pub const PLATFORM_DISCORD: &str = "discord";
pub const PLATFORM_SLACK: &str = "slack";

// 函数
pub fn default_capabilities() -> PlatformCapabilities
```

**导出类型**（来自 `capabilities.rs`）：
`CapabilityLevel`, `FileUploadCapability`, `MessageUpdateCapability`, `ModeDecisionContext`, `PlatformCapabilities`, `PlatformCapabilityService`, `ReasoningMode`

### 2.2 platform/capabilities.rs — 平台能力检测

```rust
// 能力等级
pub enum CapabilityLevel { Full, Partial, None }

// 文件上传能力
pub enum FileUploadCapability { Full, Partial, None }

// 消息更新能力（别名）
pub type MessageUpdateCapability = CapabilityLevel;

// 卡片交互能力（别名）
pub type CardInteractionCapability = CapabilityLevel;

// 平台能力结构
pub struct PlatformCapabilities {
    pub platform: String,
    pub message_update: MessageUpdateCapability,
    pub card_interaction: CardInteractionCapability,
    pub file_upload: FileUploadCapability,
    pub message_length_limit: u32,
    pub stream_mode_support: CapabilityLevel,
    pub edit_message_support: bool,
}

// 平台能力检测服务
pub struct PlatformCapabilityService { ... }

impl PlatformCapabilityService {
    pub fn new() -> Self
    pub fn get_capabilities(&self, platform: &str) -> PlatformCapabilities
    pub fn supports_mode(&self, platform: &str, mode: ReasoningMode) -> bool
    pub fn supports_mode_fully(&self, platform: &str, mode: ReasoningMode) -> bool
    pub fn get_fallback_mode(&self, platform: &str, requested_mode: ReasoningMode) -> ReasoningMode
}

// 推理模式（从 session::persistence re-export）
pub enum ReasoningMode { Direct, Plan, Stream, Hidden }

// 模式决策上下文
pub struct ModeDecisionContext {
    pub requested_mode: Option<ReasoningMode>,
    pub session_id: String,
    pub metadata: HashMap<String, String>,
}
impl ModeDecisionContext {
    pub fn new(session_id: impl Into<String>) -> Self
    pub fn with_requested_mode(self, mode: ReasoningMode) -> Self
    pub fn with_metadata(self, key: impl Into<String>, value: impl Into<String>) -> Self
}
```

**已知平台能力矩阵**：

| 平台 | message_update | card_interaction | file_upload | stream_mode_support | message_length_limit |
|------|---------------|-----------------|-------------|---------------------|---------------------|
| feishu | Partial | Full | Full | Partial | 10000 |
| telegram | Full | Partial | Full | Full | 4096 |
| discord | Full | Partial | Full | Full | 2000 |
| slack | Full | Partial | Full | Full | 3000 |
| unknown | None | None | None | None | 4000 |

**降级规则**：
- `ReasoningMode::Stream` + `CapabilityLevel::None` → `ReasoningMode::Direct`
- `ReasoningMode::Stream` + `CapabilityLevel::Partial` → `ReasoningMode::Plan`
- `ReasoningMode::Direct/Plan/Hidden` → 不降级

---

## 3. platform/feishu/ — 飞书适配器

### 3.1 feishu/mod.rs — 模块入口

嵌入设计文档（`//!` 注释），包含 Issue #161 的完整设计规范。本模块以此设计为准。

### 3.2 feishu/adapter.rs — 飞书适配器核心

```rust
// 消息服务接口（trait，需外部实现）
#[async_trait]
pub trait FeishuMessageService: Send + Sync {
    async fn send_message(&self, content: &str) -> Result<String, FeishuAdapterError>;
    async fn update_message(&self, message_id: &str, content: &str) -> Result<(), FeishuAdapterError>;
    async fn send_card(&self, card_config: &PlanCardConfig) -> Result<String, FeishuAdapterError>;
}

// 降级结果
pub struct FallbackResult {
    pub initial_message_id: String,
    pub card_message_id: String,
    pub final_message_id: String,
}

// 飞书适配器
pub struct FeishuAdapter { ... }

impl FeishuAdapter {
    pub fn new(
        capability_service: Arc<PlatformCapabilityService>,
        card_service: Arc<dyn CardService>,
        message_service: Arc<dyn FeishuMessageService>,
    ) -> Self

    /// 判断指定推理模式是否需要降级
    pub fn should_fallback(&self, mode: ReasoningMode) -> bool

    /// 获取 Stream 在飞书上的降级目标
    pub fn get_fallback_mode(&self) -> ReasoningMode  // 恒返回 ReasoningMode::Plan

    /// 降级流程是否启用（当前硬编码 true）
    pub fn is_fallback_enabled(&self) -> bool

    /// 执行 Stream→Plan 降级完整流程
    pub async fn execute_fallback(&self, intent: &ModeSwitchEvent) -> Result<FallbackResult, FeishuAdapterError>

    /// 处理模式切换事件，自动判断是否降级
    pub async fn handle_mode_switch(&self, event: &ModeSwitchEvent) -> Result<Option<FallbackResult>, FeishuAdapterError>
}
```

**降级触发条件**：`should_fallback(ReasoningMode::Stream)` 当且仅当 `stream_mode_support != Full`（即 Partial 或 None）。

### 3.3 feishu/fallback.rs — 降级步骤定义

```rust
pub enum FallbackAction {
    SendInitialMessage,
    CreateCard,
    UpdateCard,
    SendFinal,
}

pub struct FallbackStep {
    pub step: u32,
    pub action: FallbackAction,
    pub content: &'static str,
    pub persist: bool,
}

impl FallbackStep {
    pub fn content_string(&self) -> String;  // 将 content 字段转换为格式化字符串（如"🔍 进入深度分析模式..."）
}

pub const FEISHU_STREAM_FALLBACK_STEPS: &[FallbackStep]
pub fn get_fallback_steps(mode: &str) -> Option<&'static [FallbackStep]>
```

默认 4 步：发送初始提示 → 创建卡片 → 更新卡片 → 发送最终结论。

### 3.4 feishu/card.rs — Plan 卡片结构

```rust
pub struct PlanCardConfig {
    pub title: String,
    pub sections: Vec<PlanSection>,
    pub show_progress: bool,
    pub show_step_buttons: bool,
}

pub struct PlanSection {
    pub step_number: u32,
    pub title: String,
    pub content: String,
    pub status: StepStatus,
}

pub enum StepStatus { Pending, Active, Completed }

pub fn build_initial_sections(goal: Option<&str>) -> Vec<PlanSection>  // 默认 3 节：需求分析/技术方案/实现路径
pub fn default_plan_card_config(goal: Option<&str>) -> PlanCardConfig
```

### 3.5 feishu/card_updater.rs — 卡片更新服务接口

```rust
pub struct SectionUpdate {
    pub title: Option<String>,
    pub content: Option<String>,
    pub status: Option<StepStatus>,
}

pub struct CardHandle { pub message_id: String }

pub struct CardServiceError { pub message: String }

#[async_trait]
pub trait CardService: Send + Sync {
    async fn create_card(&self, config: &PlanCardConfig) -> Result<CardHandle, FeishuAdapterError>;
    async fn update_section(&self, card_id: &str, section_index: usize, update: SectionUpdate) -> Result<(), FeishuAdapterError>;
    async fn update_progress(&self, card_id: &str, current_step: u32, total_steps: u32) -> Result<(), FeishuAdapterError>;
    async fn mark_step_complete(&self, card_id: &str, step_number: u32) -> Result<(), FeishuAdapterError>;
    async fn update_card(&self, card_id: &str, config: &PlanCardConfig) -> Result<(), FeishuAdapterError>;
}
```

**注意**：`CardService` 和 `FeishuMessageService` 均为 trait，代码库中无具体实现。它们是接口定义，供外部（如 gateway/im 模块）注入实现。

### 3.6 feishu/complexity.rs — 高复杂度判断

```rust
pub fn is_high_complexity(intent: &ModeSwitchEvent) -> bool

// 判断标准：parsed_goal 中复杂关键词出现 ≥2 次，或长度 > 100 字符
// 关键词：系统、架构、设计、实现、重构、迁移

pub struct HighComplexityConfig {
    pub show_progress_bar: bool,
    pub show_key_decision_points: bool,
    pub enable_mind_map_export: bool,
    pub enable_step_confirmation: bool,
}

pub fn get_high_complexity_config() -> HighComplexityConfig
```

### 3.7 feishu/error.rs — 错误类型

```rust
pub enum FeishuAdapterError {
    CardService(String),
    MessageService(String),
    CapabilityService(String),
    FallbackNotEnabled,
    CardNotFound(String),
    InvalidModeSwitchEvent,
    SectionNotFound(usize),
    Io(std::io::Error),
    Serialization(serde_json::Error),
}
```

### 3.8 feishu/updater.rs — 逐步更新逻辑

```rust
pub async fn run_streaming_with_card_update<C: CardService + ?Sized>(
    card_service: &C,
    card: &CardHandle,
    intent: &ModeSwitchEvent,
    config: HighComplexityConfig,
) -> Result<(), FeishuAdapterError>

pub async fn update_card_content<C: CardService + ?Sized>(
    card_service: &C,
    card_id: &str,
    section_index: usize,
    content: &str,
) -> Result<(), FeishuAdapterError>
```

---

## 4. 行为规范

### 4.1 降级决策

`FeishuAdapter::should_fallback` 使用 `supports_mode_fully`（而非 `supports_mode`）：
- `stream_mode_support == Full` → 不降级
- `stream_mode_support == Partial` → 降级到 Plan
- `stream_mode_support == None` → 降级到 Direct

### 4.2 能力默认值

未知平台所有能力为 `None`，`message_length_limit` 默认为 4000。

### 4.3 ReasoningMode 重新导出

`platform` 模块从 `session::persistence::ReasoningMode` 重新导出，所有子模块使用同一类型。

---

## 5. 偏差记录

见 `SPEC_ALIGNMENT_PLAN.md` Round 07 偏差追踪表。
