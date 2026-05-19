# 斜杠指令系统

## 概述

斜杠指令系统提供统一的系统控制指令机制：以 `/` 开头的用户消息在 Gateway 层被拦截，不进入 LLM 对话流程，直接分派给对应的 Handler 执行并回复。

## 架构

斜杠指令系统由三个核心组件组成：

- **Dispatcher**：嵌入 Gateway 的指令分派器。Gateway 收到入站消息后，若内容以 `/` 开头则拦截并交给 Dispatcher，否则正常路由到 Session。
- **HandlerRegistry**：指令注册表，维护指令名到 Handler 的映射。Gateway 初始化时注册所有 Handler，查询为 O(1)。
- **Handler**：指令处理器。每个 Handler 负责一组关联指令，接收指令参数和上下文，返回 SlashResult。

Dispatcher 不持有 Session 引用——Handler 返回 SlashResult 后，由 Gateway 根据结果类型调用 Session 对应方法执行副作用。

```
用户消息到达 Gateway
  ↓
是否以 / 开头？
  ├── 否 → 正常路由到 Session
  └── 是 → Dispatcher 解析指令名 + 参数
            ↓
          HandlerRegistry 查表
            ↓
          Handler 处理 → SlashResult
            ↓
          Gateway 根据结果类型执行副作用并回复
```

部分指令支持 Immediate 模式——LLM 正在运行时也能立即响应，不被消息队列阻塞。非 Immediate 指令在 LLM 忙碌时回复等待提示。

### Handler 清单

| Handler | 负责指令 | 结果类型 | Immediate |
|---------|---------|---------|-----------|
| ModeSwitchHandler | plan, mode | SetMode / Reply | ❌ |
| SessionHandler | new, stop | NewSession / Stop | stop ✅ |
| StatusHandler | status | Reply | ✅ |
| CompactHandler | compact | Compact | ❌ |
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
            │           ├── Reply(text)        → 直接回复用户
            │           ├── SetMode(mode)      → session.set_mode() + 回复
            │           ├── NewSession         → 创建新 session + 回复
            │           ├── Stop               → 终止 run + 子 agent + 回复
            │           ├── Compact{...}       → 执行压缩 + 回复
            │           ├── SystemAppend(...)  → 更新追加区 + 回复
            │           ├── Exec{command}      → 提交 Gateway 调用 Permission 模块审批
            │           └── Unknown(cmd)       → 回复"未知指令"
            └── 未命中 → SlashResult::Unknown → 回复"未知指令"
```

关键判断点：
- 是否 `/` 开头 → 决定走斜杠指令还是 LLM 对话
- Immediate 标记 → 决定是否可绕过消息队列立即执行
- SlashResult 类型 → 决定 Gateway 执行哪种副作用

## 模块关系

- **上游**：Gateway（入站消息处理）。Gateway 在消息路由前检查 `/` 前缀并分派。
- **下游**：
  - Session 模块 — 模式切换、会话创建/停止、上下文压缩、system prompt 追加区管理、工作目录设置
  - Agent 模块 — `/stop` 终止子 agent
- **间接下游**（通过 Gateway 调用）：
  - Permission 模块 — `/exec` 和 `/git` 写操作的权限审批（由 Gateway 读取 Exec 结果后调用）
- **无关**：Processor 链（斜杠指令在 Processor 之前被 Gateway 拦截，不进入 LLM 处理流程）
