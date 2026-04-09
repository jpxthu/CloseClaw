# Spec: 斜杠指令体系 (Slash Command Handler)

## Issue
GitHub Issue #162: 【147-D】斜杠指令体系：/plan /code /review /debug /direct

## Status
**Phase**: Spec → Coding

## Background
Issue #162 implements a slash command system for explicit reasoning mode switching, complementing the implicit NL intent recognition from #160.

**Dependencies**:
- #160 (推理模式基础框架) - Provides `SLASH_MODE_MAP`, `decide_mode()`, NL intent recognition

## Current State (Partial Implementation)

The Rust codebase already has:
- `src/mode/slash_command.rs` - Core slash command parsing (case-insensitive)
- `src/mode/natural_language.rs` - NL intent recognition with confidence scoring
- `src/mode/decision.rs` - Mode decision tree with priority: slash > NL > platform

**What's Implemented**:
- ✅ Slash command parsing (`/plan`, `/code`, `/review`, `/debug`, `/direct`, `/think`)
- ✅ NL confidence-based mode inference (threshold: 0.8)
- ✅ Platform adaptation fallback

**What's MISSING**:
- ❌ `/mode` command (show current mode / switch mode)
- ❌ `/compact` command (context compression with stats)
- ❌ `/help` command (return help text)
- ❌ Analytics tracking for command usage
- ❌ Friendly error for unknown commands

## Implementation Plan

### 1. Extend SlashCommand to include handler support

Modify `src/mode/slash_command.rs`:

```rust
/// Slash command handler result
#[derive(Debug, Clone)]
pub enum CommandHandlerResult {
    /// Switch to target mode
    SwitchMode(ReasoningMode),
    /// Return text response
    Text(String),
    /// Compact context
    Compact { before: usize, after: usize },
    /// Show help
    Help(String),
    /// Unknown command
    Unknown(String),
}

/// Handle a slash command and return result
pub fn handle_slash_command(
    cmd: &SlashCommand,
    session: &SessionContext,
) -> CommandHandlerResult {
    match cmd.command.as_str() {
        "/help" => CommandHandlerResult::Help(build_help_text()),
        "/mode" => handle_mode_command(&cmd.args, session),
        "/compact" => handle_compact_command(session),
        "/plan" | "/code" | "/review" | "/debug" | "/direct" | "/think" => {
            CommandHandlerResult::SwitchMode(cmd.target_mode)
        }
        _ => CommandHandlerResult::Unknown(cmd.command.clone()),
    }
}
```

### 2. Implement /mode command

```rust
fn handle_mode_command(args: &str, session: &SessionContext) -> CommandHandlerResult {
    if args.is_empty() {
        // Show current mode
        let current = session.get_current_mode();
        CommandHandlerResult::Text(format!("当前模式：{}", current))
    } else {
        // Switch mode
        let target = match args.to_lowercase().as_str() {
            "direct" => ReasoningMode::Direct,
            "plan" => ReasoningMode::Plan,
            "stream" => ReasoningMode::Stream,
            "hidden" => ReasoningMode::Hidden,
            _ => return CommandHandlerResult::Text(
                format!("无效模式。可用模式：direct, plan, stream, hidden")
            ),
        };
        CommandHandlerResult::SwitchMode(target)
    }
}
```

### 3. Implement /compact command

```rust
fn handle_compact_command(session: &SessionContext) -> CommandHandlerResult {
    let before = session.get_message_count();
    let summary = session.compact();
    let after = session.get_message_count();
    CommandHandlerResult::Compact { before, after }
}
```

### 4. Implement /help command

```rust
fn build_help_text() -> String {
    r#"可用斜杠指令：
/plan <任务>   - 先规划再执行
/code <任务>   - 生成代码
/review <内容> - 代码审查
/debug <问题>  - 调试分析
/direct        - 直接回答
/think <问题>  - 深度思考
/mode          - 查看当前模式
/mode <模式>   - 切换到指定模式
/compact       - 压缩上下文
/help          - 显示此帮助"#.to_string()
}
```

### 5. Analytics tracking

Add analytics event emission in `handle_slash_command`:

```rust
// Track command usage
analytics::track("slash_command", event!{
    "command": cmd.command,
    "target_mode": format!("{:?}", cmd.target_mode),
    "is_meta": cmd.is_meta_command,
});
```

### 6. Unknown command handling

When `CommandHandlerResult::Unknown` is returned, send a friendly message:
- "未知指令: {command}。输入 /help 查看可用指令。"

## Acceptance Criteria

| # | Criterion | Status |
|---|-----------|--------|
| 1 | `/plan`, `/code`, `/review`, `/debug`, `/direct`, `/think` 正确触发对应推理模式 | ✅ |
| 2 | `/mode` 无参数时返回当前模式，有参数时切换模式 | TODO |
| 3 | `/compact` 正确压缩上下文并返回压缩统计 | TODO |
| 4 | `/help` 返回完整的指令帮助文本 | TODO |
| 5 | 自然语言触发置信度 > 0.8 时自动推断模式（无感切换） | ✅ |
| 6 | 未知指令返回友好提示，不报错 | TODO |
| 7 | 指令解析不区分大小写（`/PLAN` 和 `/plan` 等效） | ✅ |
| 8 | 每个指令的使用情况被记录到 analytics | TODO |

## Files to Modify

1. `src/mode/slash_command.rs` - Add handler functions
2. `src/session/mod.rs` - Add `get_current_mode()`, `compact()` methods
3. `src/analytics/mod.rs` - Add slash command tracking

## Verification

Run tests:
```bash
cargo test --lib -- slash_command
cargo test --lib -- decision
cargo test --test integration
```
