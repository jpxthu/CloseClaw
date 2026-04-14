# Mode 模块规格说明书

## 模块概述

Mode 模块根据用户输入决策当前对话的推理模式（ReasoningMode），并在模式切换时发布 `ModeSwitchEvent` 事件。模式切换优先级：**斜杠指令 > 自然语言 > 平台适配**。

模块内部划分为：斜杠指令解析、自然语言意图识别、模式决策、切换事件四个子模块。

---

## 公开接口

### 构造

- `ModeDecisionTree::new(capability_service) → Self` — 构造决策树实例
- `NaturalLanguagePatterns::new() → Self` — 构造中文模式库（plan/code/debug/review 四类关键词）

### 主操作

- `decide_mode(user_input, platform, context, capability_service) → ReasoningMode` — 顶层模式决策入口
- `ModeDecisionTree::decide(&self, user_input, platform, context) → ReasoningMode` — 决策树方法形式
- `parse_slash_command(input) → Option<SlashCommand>` — 解析斜杠指令
- `handle_slash_command(cmd) → SlashCommandResult` — 执行斜杠指令
- `parse_natural_language_intent(input) → IntentResult` — 解析自然语言意图
- `format_mode(mode) → &'static str` — 格式化模式为可读字符串
- `unknown_command_response(command) → String` — 生成未知指令的提示文本

### 查询

- `SLASH_MODE_MAP` — 斜杠指令到推理模式的静态映射表
- `NL_CONFIDENCE_THRESHOLD` — 自然语言触发置信度阈值（0.8）

### 清理

无

---

## 架构与结构

### 子模块划分

| 子模块 | 职责 |
|--------|------|
| `slash_command` | 斜杠指令解析（`/` 开头）、指令执行、帮助文本 |
| `natural_language` | 自然语言意图识别（中文关键词匹配） |
| `decision` | 顶层决策逻辑（优先级链：slash → NL → platform fallback） |
| `switch_event` | 模式切换事件构造与序列化 |

### 决策流程

```
decide_mode(user_input)
  ├─ parse_slash_command → 非元指令？ → 直接返回 target_mode
  ├─ parse_natural_language_intent → 置信度 ≥ 0.8？ → platform fallback → 返回
  └─ context.requested_mode → 平台不支持？ → fallback → 返回
  └─ 默认 → ReasoningMode::Direct
```

### 数据流

1. 用户输入 → `parse_slash_command`（斜杠指令）
2. 未命中 → `parse_natural_language_intent`（自然语言）
3. 推断出模式 → `PlatformCapabilityService::supports_mode`（平台适配）
4. 最终决策 → `ReasoningMode`
5. 决策结果 → `ModeSwitchEvent::from_slash / from_natural_language / from_platform_adaptation`

---

## 跨模块格式

- **决策上下文**：`ModeDecisionContext`（`crate::platform::capabilities`），含 `requested_mode` 字段
- **平台能力**：`PlatformCapabilityService`（`crate::platform::capabilities`），含 `supports_mode`、`get_fallback_mode`
- **模式枚举**：`ReasoningMode`（`crate::session::persistence`），含 `Direct/Plan/Stream/Hidden`
- **事件时间戳**：`chrono::DateTime<Utc>`

---

## 斜杠指令解析

### 核心类型

**`SlashCommand`** — 解析后的斜杠指令
- `command: String` — 指令名（如 `/plan`）
- `args: String` — 参数部分
- `raw_input: String` — 原始输入
- `target_mode: ReasoningMode` — 对应目标模式
- `is_meta_command: bool` — 是否为元指令（不触发模式切换）

**`SlashCommandResult`** — 斜杠指令执行结果
- `SwitchMode(ReasoningMode)` — 切换到目标模式
- `Text(String)` — 返回文本响应
- `Compact { before: usize, after: usize }` — 上下文压缩占位结果
- `Unknown(String)` — 未知指令

**`SlashModeMap`** — 指令映射表结构体，内部维护 HashMap，支持 `get`、`is_meta`、`matches` 方法。

### 工厂方法与常量

- `SLASH_HELP_TEXT` — `/help` 返回的帮助文本常量
- `unknown_command_response(command) → String` — 未知指令的提示文本

### 支持的指令

| 指令 | 行为 |
|------|------|
| `/plan` | 切换到 Plan 模式 |
| `/code` | 切换到 Stream 模式 |
| `/review` | 切换到 Plan 模式 |
| `/debug` | 切换到 Stream 模式 |
| `/direct` | 切换到 Direct 模式 |
| `/think` | 切换到 Hidden 模式 |
| `/mode [arg]` | 无参数返回用法提示；有参数切换到指定模式 |
| `/compact` | 返回上下文压缩占位结果 `{before:0, after:0}`，实际压缩由调用方实现 |
| `/help` | 返回帮助文本 |
| 未知指令 | 返回 `SlashCommandResult::Unknown` |

### 行为规范

- 指令匹配**不区分大小写**
- 元指令（`/mode`、`/compact`、`/help`）处理后继续走自然语言判断流程（不直接切模式）
- `parse_slash_command` 在输入不以 `/` 开头时返回 `None`

---

## 自然语言意图识别

### 核心类型

**`IntentResult`** — 意图识别结果，含 `mode`、`confidence`、`matched_keywords` 字段，方法 `is_confident(threshold) → bool`。

**`NaturalLanguagePatterns`** — 模式库，提供 `recognize_plan`、`recognize_code`、`recognize_debug`、`recognize_review` 四个识别方法。

### 模式映射

| 识别类型 | 目标模式 |
|---------|---------|
| plan / review | `ReasoningMode::Plan` |
| code / debug | `ReasoningMode::Stream` |

### 置信度计算

`min(matched_count × 0.85, 1.0)`，阈值 0.8。

关键词匹配**不区分大小写**。无任何匹配时返回 `mode: Direct, confidence: 0.0, matched_keywords: []`。

---

## 模式切换事件

### 核心类型

**`UserIntent`** — 附在事件上的用户意图，含 `raw_input`（原始输入）和 `parsed_goal`（解析后的目标描述）字段。

**`ModeSwitchTrigger`** — 触发类型枚举，变体：`SlashCommand`、`NaturalLanguage`、`Auto`、`UserRequest`。序列化时使用 snake_case。

**`ModeSwitchEvent`** — 模式切换事件，字段：`event_type`（固定 `"mode_switch"`）、`session_id`、`timestamp`、`from_mode`、`to_mode`、`trigger`、`trigger_value`、`user_intent`。

### 工厂方法

- `ModeSwitchEvent::new(session_id, from_mode, to_mode, trigger, trigger_value) → Self`
- `ModeSwitchEvent::with_user_intent(self, intent) → Self` — 链式附加用户意图
- `ModeSwitchEvent::from_slash(session_id, from_mode, to_mode, command, raw_input, args) → Self`
- `ModeSwitchEvent::from_natural_language(session_id, from_mode, to_mode, keyword, raw_input) → Self`
- `ModeSwitchEvent::from_platform_adaptation(session_id, from_mode, to_mode, platform) → Self`
