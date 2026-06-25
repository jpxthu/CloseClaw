# 斜杠指令系统

## 概述

斜杠指令系统提供统一的系统控制指令机制：以 `/` 开头的用户消息在 Gateway 层被拦截，不进入 LLM 对话流程，直接分派给对应的 Handler 执行并回复。

## 架构

斜杠指令系统由三个核心组件组成：

- **SlashDispatcher**：嵌入 Gateway 的指令分派器。Gateway 收到入站消息后，若内容以 `/` 开头则拦截并交给 SlashDispatcher，否则正常路由到 Session。
- **HandlerRegistry**：指令注册表，维护指令名到 Handler 的映射。Gateway 初始化时注册所有 Handler。
- **Handler**：指令处理器。每个 Handler 负责一组关联指令，接收指令参数和上下文，返回 SlashResult。

SlashDispatcher 不持有 Session 引用——Handler 返回 SlashResult 后，由 Gateway 构造 SideEffectContext 并调用 SlashResult.execute()。SideEffectContext 封装 Session 操作和消息回复能力，各 SlashResult 变体在 execute() 内自行完成副作用。Gateway 不感知具体变体，只负责传递上下文。

```
用户消息到达 Gateway
  ↓
是否以 / 开头？
  ├── 否 → 正常路由到 Session
  └── 是 → SlashDispatcher 解析指令名 + 参数
            ↓
          HandlerRegistry 查表
            ↓
          Handler 处理 → SlashResult
            ↓
          Gateway 构造 SideEffectContext → SlashResult.execute(ctx)
```

部分指令支持 Immediate 模式——LLM 正在运行时也能立即响应，不被 Session 忙碌队列阻塞。非 Immediate 指令在 LLM 忙碌时回复等待提示。

### Handler 清单

| Handler | 负责指令 | 结果类型 | Immediate |
|---------|---------|---------|-----------|
| ModeSwitchHandler | plan, mode | SetMode / Reply | ❌ |
| NewSessionHandler | new | NewSession | ❌ |
| StopHandler | stop | Stop | ✅ |
| StatusHandler | status | Reply | ✅ |
| CompactHandler | compact | Compact | ❌ |
| ReasoningHandler | reasoning | SetReasoning / Reply | ✅ |
| VerboseHandler | verbose | SetVerbosity / Reply | ✅ |
| SystemHandler | system | SystemAppend / Reply | ❌ |
| WorkdirHandler | cd, pwd, git | Reply / Exec | ❌ |
| ExecHandler | exec | Exec / Reply | ❌ |
| HelpHandler | help | Reply | ✅ |

### 子功能目录

- [上下文压缩](compact.md) — `/compact` 触发对话历史压缩
- [命令执行](exec.md) — `/exec` owner 特权命令执行
- [帮助](help.md) — `/help` 动态生成帮助文本
- [模式切换](mode-switching.md) — `/plan` 和 `/mode`，切换 Normal/Plan 模式
- [会话管理](session-management.md) — `/new` 创建新会话，`/stop` 终止当前运行
- [推理深度控制](reasoning.md) — `/reasoning` 查询或设置推理深度
- [信息展示等级](verbose.md) — `/verbose` 查询或设置信息展示等级
- [状态查询](status.md) — `/status` 查看会话状态
- [System Prompt 追加](system-append.md) — `/system` 动态管理 system prompt 追加区
- [工作目录操作](workdir.md) — `/cd`、`/pwd`、`/git` 工作目录操作

## 数据流

```
入站消息
  ↓
Gateway.handle_inbound()
  ↓
内容以 / 开头？
  ├── 否 → route_to_session() → 正常 LLM 对话流程
  └── 是 → SlashDispatcher.dispatch()
            ↓
          parse_slash(): 分离指令名和参数
            ↓
          HandlerRegistry.get(指令名)
            ├── 命中 → handler.handle(args, ctx)
            │           ↓
            │         SlashResult 变体
            │           ↓
            │         Gateway.handle_slash_result()
            │           ↓
            │         构造 SideEffectContext（封装 session 引用 + 回复通道）
            │           ↓
            │         SlashResult.execute(ctx) —— 各变体自行完成副作用
            │           ↓
            │         回复内容经出站 Processor Chain → IM 插件渲染发送
            └── 未命中 → SlashResult::Unknown → 回复（经出站 Processor Chain → IM 插件发送）
```

关键判断点：
- 是否 `/` 开头 → 决定走斜杠指令还是 LLM 对话
- Immediate 标记 → 决定是否可绕过 Session 忙碌队列立即执行

## 模块关系

- **上游**：Gateway（入站消息处理）。Gateway 在消息路由前检查 `/` 前缀并分派。`/approve`、`/deny` 由 Gateway 层硬拦截（走审批流程验证），不进入 SlashDispatcher。
- **下游**：
  - Session 模块 — 模式切换、会话创建/停止（含级联终止子 session）、推理深度控制、上下文压缩、system prompt 追加区管理、工作目录设置
- **间接下游**（通过 Gateway 调用）：
  - Permission 模块 — `/exec` 和 `/git` 写操作的权限审批（Gateway 在收到 Handler 返回的 Exec SlashResult 后、实际执行前调用 Permission 引擎）
- **间接下游**（通过 Session 生效）：
  - LLM 模块 — `/reasoning` 写入的推理深度在下一次 LLM 调用时映射为各模型的原生参数（含不支持等级的自动降级）
- **间接相关**：Processor Chain（斜杠指令消息经入站 Processor Chain 处理后由 Gateway 路由到 SlashDispatcher；SlashResult 各变体通过 SideEffectContext 的回复通道产出回复内容，由 Gateway 送入出站 Processor Chain 处理后经 IM 插件渲染发送）
