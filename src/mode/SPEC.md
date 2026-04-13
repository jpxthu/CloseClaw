# Mode 模块规格说明书

## 模块职责

Mode 模块负责根据用户输入（显式斜杠指令或隐式自然语言）决策当前对话的推理模式（ReasoningMode），并在模式切换时发布 `ModeSwitchEvent` 事件。模式切换的优先级为：**斜杠指令 > 自然语言 > 平台适配**。

---

## 公开接口

### mod.rs — 模块入口

```rust
pub use decision::{decide_mode, ModeDecisionTree};
pub use natural_language::{parse_natural_language_intent, IntentResult, NaturalLanguagePatterns};
pub use slash_command::{
    format_mode, handle_slash_command, parse_slash_command,
    SlashCommand, SlashCommandResult, SlashModeMap,
    SLASH_HELP_TEXT, SLASH_MODE_MAP, unknown_command_response,
};
pub use switch_event::{ModeSwitchEvent, ModeSwitchTrigger, UserIntent};
```

### 常量

| 名称 | 值 | 说明 |
|------|----|------|
| `NL_CONFIDENCE_THRESHOLD` | `0.8` | 自然语言触发置信度阈值（定义于 `decision.rs`） |
| `SLASH_HELP_TEXT` | `&str` | `/help` 返回的帮助文本 |

---

## 核心数据结构

### `ReasoningMode`（定义于 `src/session/persistence`）

```rust
pub enum ReasoningMode {
    Direct,  // 直接回答，无规划
    Plan,    // 规划模式（先规划再执行）
    Stream,  // 流式输出（生成代码/调试等）
    Hidden,  // 隐藏思考过程（深度思考）
}
```

### `SlashCommand`（`slash_command.rs`）

```rust
pub struct SlashCommand {
    pub command: String,           // 指令原文，如 "/plan"
    pub args: String,              // 指令参数（不含命令本身）
    pub raw_input: String,        // 用户原始输入
    pub target_mode: ReasoningMode, // 对应推理模式
    pub is_meta_command: bool,     // 是否为元指令（不切换模式）
}

impl SlashCommand {
    pub fn new(
        command: impl Into<String>,
        args: impl Into<String>,
        raw_input: impl Into<String>,
        target_mode: ReasoningMode,
        is_meta_command: bool,
    ) -> Self;
}
```

### `SlashCommandResult`（`slash_command.rs`）

```rust
pub enum SlashCommandResult {
    SwitchMode(ReasoningMode),    // 切换到目标模式
    Text(String),                 // 返回文本响应（如 /help）
    Compact { before: usize, after: usize }, // 上下文压缩结果
    Unknown(String),              // 未知指令
}
```

### `IntentResult`（`natural_language.rs`）

```rust
pub struct IntentResult {
    pub mode: ReasoningMode,       // 推断出的目标模式
    pub confidence: f32,           // 置信度 0.0~1.0
    pub matched_keywords: Vec<String>, // 命中的关键词列表
}

impl IntentResult {
    pub fn new(mode: ReasoningMode, confidence: f32, matched_keywords: Vec<String>) -> Self;
    /// 置信度是否达到自然语言触发阈值
    pub fn is_confident(&self, threshold: f32) -> bool { self.confidence >= threshold }
}
```

### `NaturalLanguagePatterns`（`natural_language.rs`）

```rust
pub struct NaturalLanguagePatterns {
    plan_patterns: Vec<(&'static str, f32)>,
    code_patterns: Vec<(&'static str, f32)>,
    debug_patterns: Vec<(&'static str, f32)>,
    review_patterns: Vec<(&'static str, f32)>,
}

impl NaturalLanguagePatterns {
    pub fn new() -> Self;   // 返回默认中文模式库
    pub fn recognize_plan(&self, text: &str) -> bool;
    pub fn recognize_code(&self, text: &str) -> bool;
    pub fn recognize_debug(&self, text: &str) -> bool;
    pub fn recognize_review(&self, text: &str) -> bool;
}
```

### `ModeSwitchTrigger`（`switch_event.rs`）

```rust
pub enum ModeSwitchTrigger {
    SlashCommand,    // 显式斜杠指令触发
    NaturalLanguage, // 自然语言隐式触发
    Auto,            // 平台自动适配触发
    UserRequest,     // 用户主动请求
}
```

### `ModeSwitchEvent`（`switch_event.rs`）

```rust
pub struct ModeSwitchEvent {
    pub event_type: String,           // 固定值 "mode_switch"
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub from_mode: ReasoningMode,
    pub to_mode: ReasoningMode,
    pub trigger: ModeSwitchTrigger,
    pub trigger_value: String,
    pub user_intent: Option<UserIntent>,
}

pub struct UserIntent {
    pub raw_input: String,     // 原始用户输入
    pub parsed_goal: String,  // 解析后的目标描述
}
```

---

## 行为规范

### 斜杠指令解析（`slash_command.rs`）

**支持的指令**：

| 指令 | 目标模式 | 类型 |
|------|---------|------|
| `/plan` | Plan | 模式切换 |
| `/code` | Stream | 模式切换 |
| `/review` | Plan | 模式切换 |
| `/debug` | Stream | 模式切换 |
| `/direct` | Direct | 模式切换 |
| `/think` | Hidden | 模式切换 |
| `/mode` | — | 元指令（查看/切换模式） |
| `/compact` | — | 元指令（压缩上下文） |
| `/help` | — | 元指令（返回帮助文本） |

**行为规范**：

- 解析**不区分大小写**，`/PLAN` 与 `/plan` 等效
- 斜杠指令以 `/` 开头，后跟指令名和可选参数（以空格分隔）
- `parse_slash_command()` 在输入不以 `/` 开头时返回 `None`
- `/mode` 无参数时返回用法提示；有参数时根据参数值切换模式
- `/help` 返回 `SLASH_HELP_TEXT` 常量文本
- `/compact` 返回 `SlashCommandResult::Compact { before: 0, after: 0 }`（实际压缩逻辑由调用方实现）
- 未知指令返回 `SlashCommandResult::Unknown(command)`

**`SLASH_MODE_MAP`**：

```rust
pub const SLASH_MODE_MAP: &[(&str, ReasoningMode)] = &[
    ("/plan", ReasoningMode::Plan),
    ("/code", ReasoningMode::Stream),
    ("/review", ReasoningMode::Plan),
    ("/debug", ReasoningMode::Stream),
    ("/direct", ReasoningMode::Direct),
    ("/think", ReasoningMode::Hidden),
];
```

### 自然语言意图识别（`natural_language.rs`）

**意图模式列表**：

| 意图类型 | 目标模式 | 关键词示例 |
|---------|---------|-----------|
| 规划意图 | Plan | 帮我规划、怎么设计、设计一个、规划一下、有什么方案、请分析一下 |
| 编码意图 | Stream | 写代码、如何实现、代码示例、写个函数、写一个、帮我写、生成代码 |
| 调试意图 | Stream | 为什么报错、为什么报、怎么修复、问题排查、调试、报错、出错了 |
| 审查意图 | Plan | 检查一下、有什么问题、代码审查、review、审视、看看代码 |

**置信度计算**：`匹配关键词数 × 0.85`（上限 1.0）

**行为规范**：
- `parse_natural_language_intent()` 对四种意图并行打分，取置信度最高者
- 无任何关键词匹配时返回 `IntentResult { mode: Direct, confidence: 0.0, matched_keywords: [] }`
- 关键词匹配**不区分大小写**（先将输入和关键词都转为小写再匹配）
- 同一关键词在多种意图中出现时，取最高置信度的意图

### 模式决策（`decision.rs`）

**`decide_mode()` 决策流程**（优先级依次降低）：

1. **斜杠指令**：`parse_slash_command()` 命中非元指令时，直接返回对应 `target_mode`
2. **自然语言**：置信度 ≥ `NL_CONFIDENCE_THRESHOLD`（0.8）时，返回推断模式并经平台适配
3. **平台适配**：`context.requested_mode` 存在但平台不支持时，返回 fallback 模式
4. **默认**：返回 `ReasoningMode::Direct`

**`ModeDecisionTree.decide()` 流程**与 `decide_mode()` 相同，仅接口形式不同（面向对象 vs 函数式）。

**`ModeDecisionTree` 构造器**：

```rust
impl ModeDecisionTree {
    pub fn new(capability_service: PlatformCapabilityService) -> Self;
}
```

### 模式切换事件（`switch_event.rs`）

**工厂方法**：

- `ModeSwitchEvent::new(...)` — 通用构造器
- `ModeSwitchEvent::from_slash(...)` — 从斜杠指令创建，含 `UserIntent`
- `ModeSwitchEvent::from_natural_language(...)` — 从自然语言创建，含 `UserIntent`
- `ModeSwitchEvent::from_platform_adaptation(...)` — 从平台适配创建

**序列化**：`trigger` 字段以 snake_case 序列化为 JSON（如 `"slash_command"`、`"natural_language"`、`"auto"`、`"user_request"`）。

---

## 模块边界

| 依赖模块 | 依赖内容 |
|---------|---------|
| `crate::session::persistence::ReasoningMode` | 模式枚举定义 |
| `crate::platform::capabilities::PlatformCapabilityService` | 平台能力查询（`supports_mode`、`get_fallback_mode`） |
| `crate::platform::capabilities::ModeDecisionContext` | 决策上下文（`requested_mode` 字段） |
| `chrono::DateTime<Utc>` | switch_event 中时间戳类型 |

---

## 偏差分析（代码 vs 文档）

### 1. `SlashCommandResult` vs `CommandHandlerResult`（SLASH_COMMAND_HANDLER.md）

**文档**（Issue #162）定义的枚举名为 `CommandHandlerResult`，包含 `SwitchMode`、`Text`、`Compact`、`Help`、`Unknown` 五种变体。

**代码**实际实现的枚举名为 `SlashCommandResult`，包含 `SwitchMode`、`Text`、`Compact`、`Unknown` 四种变体。`Help` 变体不存在，帮助文本通过 `Text` 变体返回。

**影响**：轻微。枚举名不同但语义完全兼容，`Help` 变体的缺失不影响功能（`Text` 变体可携带帮助文本）。

### 2. `/compact` 返回值的占位性质（SLASH_COMMAND_HANDLER.md）

**文档**描述 `/compact` 应执行实际的上下文压缩并返回真实统计（压缩前后 token 数）。

**代码**中 `handle_slash_command()` 对 `/compact` 的处理返回固定的占位值 `{ before: 0, after: 0 }`，注释注明"实际压缩逻辑由调用方实现"。

**影响**：文档描述的是最终期望行为，代码当前为占位实现。这是合理的**待完成状态**，非 bug。

### 3. `UserRequest` 变体未使用（REASONING_MODE_FRAMEWORK.md）

**文档**定义了 `ModeSwitchTrigger::UserRequest`，`switch_event.rs` 代码中该变体存在，但在 `decision.rs` 的决策逻辑中没有任何路径产生此触发类型。

**影响**：无功能影响。`ModeSwitchTrigger` 为可扩展枚举，保留 `UserRequest` 作为未来扩展预留。

### 4. 测试覆盖差异（SLASH_COMMAND_HANDLER.md）

**文档** Acceptance Criteria 第 8 条（analytics 追踪）和第 2 条（`/mode` 无参返回当前模式）标记为 TODO。

**代码**：`/mode` 无参时返回用法提示而非当前模式（不同于文档描述）；analytics 追踪未实现。

**影响**：`/mode` 行为与文档描述不符——文档说无参时显示当前模式，代码返回"请提供模式：..."。`/mode` 的完整模式切换功能（含参数）已实现。

### 5. 文档冗余（REASONING_MODE_FRAMEWORK.md）

文档中包含大量实现细节（文件结构、验收标准、"实现细节"章节），这些内容属于开发记录而非规格描述，与 SPEC 规范要求的"精确功能说明"有偏差。

**影响**：非功能偏差，不影响实现正确性。文档中真正构成规格的是类型定义、映射表和决策流程描述。
