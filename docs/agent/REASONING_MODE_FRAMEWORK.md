# Issue #160 Spec: 平台能力检测 + 推理模式基础框架

## 概述

为 CloseClaw 实现统一的平台能力检测机制和推理模式基础框架，作为 #161、#162、#163 等后续功能的基础依赖。

## 背景

需要建立统一的平台能力检测机制和推理模式基础框架，后续各平台适配和功能模块都依赖于此。

**依赖关系**：
- #160（本 issue）：基础框架，**无外部依赖**
- #161（飞书降级适配）：依赖 #160
- #162（斜杠指令体系）：依赖 #160
- #163（卡片交互系统）：依赖 #161

## 设计

### 平台能力矩阵

#### 平台能力类型定义 (`src/platform/capabilities.rs`)

```rust
/// Capability level enumeration
pub enum CapabilityLevel {
    Full,    // 完全支持
    Partial, // 部分支持（有限制）
    None,    // 不支持
}

/// File upload capability
pub enum FileUploadCapability {
    Full,
    Partial,
    None,
}

/// 平台能力结构
pub struct PlatformCapabilities {
    pub platform: String,              // 平台标识符
    pub message_update: CapabilityLevel,
    pub card_interaction: CapabilityLevel,
    pub file_upload: FileUploadCapability,
    pub message_length_limit: u32,      // 最大消息长度
    pub stream_mode_support: CapabilityLevel,
    pub edit_message_support: bool,    // 是否支持编辑消息
}
```

#### 已知平台能力配置

| 平台 | message_update | card_interaction | stream_mode_support | message_length_limit |
|------|----------------|------------------|---------------------|---------------------|
| feishu | Partial | Full | Partial (需降级) | 10000 |
| telegram | Full | Partial | Full | 4096 |
| discord | Full | Partial | Full | 2000 |
| slack | Full | Partial | Full | 3000 |

### 推理模式状态机

#### 模式状态 (`src/session/persistence.rs`)

```rust
pub enum ReasoningMode {
    Direct,  // 直接回答
    Plan,    // 规划模式
    Stream,  // 流式输出
    Hidden,  // 隐藏思考过程
}

pub struct ReasoningModeState {
    pub current_step: u32,       // 当前步骤（1-indexed）
    pub total_steps: u32,       // 总步骤数
    pub step_messages: Vec<String>, // 步骤输出
    pub is_complete: bool,
}
```

#### 模式切换事件 (`src/mode/switch_event.rs`)

```rust
pub enum ModeSwitchTrigger {
    SlashCommand,    // 斜杠指令触发
    NaturalLanguage, // 自然语言触发
    Auto,            // 自动适配
    UserRequest,     // 用户请求
}

pub struct UserIntent {
    pub raw_input: String,
    pub parsed_goal: String,
}

pub struct ModeSwitchEvent {
    pub event_type: String,
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub from_mode: ReasoningMode,
    pub to_mode: ReasoningMode,
    pub trigger: ModeSwitchTrigger,
    pub trigger_value: String,
    pub user_intent: Option<UserIntent>,
}
```

### 模式切换决策逻辑 (`src/mode/decision.rs`)

```rust
// 决策优先级：斜杠指令 > 自然语言 > 平台适配

pub fn decide_mode(
    user_input: &str,
    platform: &str,
    context: &ModeDecisionContext,
    capability_service: &PlatformCapabilityService,
) -> ReasoningMode {
    // 1. 显式斜杠指令 → 对应模式
    if let Some(slash_cmd) = parse_slash_command(user_input) {
        if !slash_cmd.is_meta_command {
            return slash_cmd.target_mode;
        }
    }

    // 2. 自然语言隐式触发（置信度 > 0.8）
    let nl_intent = parse_natural_language_intent(user_input);
    if nl_intent.confidence >= NL_CONFIDENCE_THRESHOLD {
        return capability_service.get_fallback_mode(platform, nl_intent.mode);
    }

    // 3. 平台能力适配
    if let Some(requested_mode) = context.requested_mode {
        if !capability_service.supports_mode(platform, requested_mode) {
            return capability_service.get_fallback_mode(platform, requested_mode);
        }
        return requested_mode;
    }

    ReasoningMode::Direct
}
```

### 斜杠指令→模式映射 (`src/mode/slash_command.rs`)

| 指令 | 目标模式 |
|------|---------|
| /plan | Plan |
| /code | Stream |
| /review | Plan |
| /debug | Stream |
| /direct | Direct |
| /think | Hidden |

元指令（不切换模式）：`/mode`, `/compact`, `/help`

### 自然语言触发配置 (`src/mode/natural_language.rs`)

置信度 = 匹配关键词数 × 0.85（上限 1.0）

| 意图 | 模式 | 关键词示例 |
|------|------|----------|
| 规划 | Plan | 帮我规划、怎么设计、设计一个 |
| 编码 | Stream | 写代码、写个函数、帮我写 |
| 调试 | Stream | 为什么报错、怎么修复、调试 |
| 审查 | Plan | 检查一下、代码审查、review |

## 文件结构

```
src/
  platform/
    mod.rs              # 模块导出和常量
    capabilities.rs     # 平台能力类型和服务
  mode/
    mod.rs              # 模块导出
    decision.rs         # 模式决策树
    slash_command.rs     # 斜杠指令解析
    switch_event.rs      # 模式切换事件
    natural_language.rs # 自然语言意图识别
```

## 验收标准

- [x] `PlatformCapabilities` 覆盖所有已知平台（feishu/telegram/discord/slack）
- [x] `PlatformCapabilityService.get_fallback_mode()` 对飞书 stream 模式返回 "plan"
- [x] `ReasoningMode` 四种模式（direct/plan/stream/hidden）状态定义完整
- [x] `ModeSwitchEvent` payload 结构符合设计，包含所有必需字段
- [x] `decide_mode()` 决策逻辑：斜杠指令 > 自然语言 > 平台适配
- [x] 模式切换事件正确发布到事件总线（Event Bus / NATS）
- [x] 自然语言触发意图识别（规划/编码/调试/审查）准确率基线达标

## 实现细节

### 平台能力服务

```rust
impl PlatformCapabilityService {
    pub fn get_capabilities(&self, platform: &str) -> PlatformCapabilities
    pub fn supports_mode(&self, platform: &str, mode: ReasoningMode) -> bool
    pub fn get_fallback_mode(&self, platform: &str, mode: ReasoningMode) -> ReasoningMode
}
```

### 斜杠指令解析

```rust
pub fn parse_slash_command(input: &str) -> Option<SlashCommand>
// 返回 Some(SlashCommand) 如果是斜杠指令，否则返回 None
```

### 自然语言意图识别

```rust
pub fn parse_natural_language_intent(input: &str) -> IntentResult
// 返回 IntentResult { mode, confidence, matched_keywords }
```

## 测试覆盖

- `src/platform/mod.rs` - 平台能力测试
- `src/mode/mod.rs` - 斜杠指令、自然语言、决策树测试
- `src/mode/slash_command.rs` - 指令解析测试
- `src/mode/natural_language.rs` - 意图识别测试
- `src/mode/switch_event.rs` - 事件结构测试
- `src/mode/decision.rs` - 决策逻辑测试
