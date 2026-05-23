# Gateway

## 概述

Gateway 是消息路由中枢。它管理所有 IM 平台的适配器，调度 Processor Chain 完成消息的出入站处理，做出路由决策（斜杠指令 vs 普通对话），并选择对应平台的 Renderer 完成出站消息的格式转换与发送。

## 架构

Gateway 由四个职责组成，不含业务逻辑：

```
=== 入站 ===

IM Adapter             平台格式解析  →  NormalizedMessage
                                            ↓
Processor Chain        RawLog(10) → SessionRouter(20) → MessageCleaner(30)
                                                                ↓
                                                      MarkdownNormalizer(40)
                                            ↓  ProcessedMessage
Gateway                路由决策
                       ├─ / 开头 → SlashDispatcher
                       └─ 普通   → Session → LLM

=== 出站 ===

Session / SlashHandler                  ContentBlock[]
                                            ↓
Processor Chain        DslParser(10) → RawLog(20)
                                            ↓  ProcessedMessage
Gateway                选择 Renderer → 渲染
                                            ↓  RenderedOutput
IM Adapter             发送 → 平台原生格式
```

- **IM Adapter 管理**：注册和维护各平台适配器。入站方向将平台原始格式归一化为统一结构，出站方向接收渲染后的 payload 并发送。
- **Processor Chain 调度**：按 priority 顺序调度入站和出站处理器链。入站链完成消息归一化和清洗，出站链完成 DSL 解析和日志记录。
- **路由决策**：根据消息前缀决定走向——以 `/` 开头则拦截分派给 SlashDispatcher，否则路由到 Session 进入 LLM 对话流程。
- **Renderer 选择**：根据目标平台选择对应 Renderer，将 ContentBlock[] 和 DSL 解析结果转换为平台原生格式。

Gateway 维护以下运行时注册表：

- **Adapter Registry**：平台 → IM Adapter 的映射
- **Renderer Registry**：平台 → Renderer 的映射
- **Processor Registry**：入站/出站处理器链，按 priority 排序

**明确不做的职责**（归属其他模块）：Session 生命周期管理、Bootstrap 加载与 System Prompt 构建、LLM 调用、Busy/Pending 状态管理。

## 数据流

### 入站路径

Gateway 收到 Processor Chain 产出的 ProcessedMessage 后，按消息内容做路由决策：

- **`/` 开头 → 斜杠指令**：分派给 SlashDispatcher。
  - Immediate 指令（`/stop`、`/status`、`/help`）→ 绕过消息队列立即执行。
  - 非 Immediate 指令 → Handler 处理，返回 SlashResult，Gateway 执行对应副作用（见下方表格）。
- **普通消息**：路由到 SessionManager → Session → LLM，返回 ContentBlock[] 进入出站链路。

> 斜杠指令的解析和 SlashResult 处理详见 [slash 模块](../slash/README.md)。

### 出站路径

Gateway 收到 ContentBlock[] 后（来源：LLM 响应或 SlashHandler），调度 Outbound Processor Chain 处理，选择目标平台 Renderer 渲染，交给 IM Adapter 发送。详见上方架构图出站部分。

### 斜杠指令副作用执行

SlashDispatcher 返回 SlashResult 后，Gateway 根据结果类型调用对应模块：

| SlashResult 类型 | Gateway 执行的动作 |
|---|---|
| Reply | 直接回复用户 |
| SetMode | 调用 Session 切换模式 |
| NewSession | 创建新 session |
| Stop | 终止当前运行（含子 agent） |
| Compact | 触发会话压缩 |
| SystemAppend | 更新 system prompt 追加区 |
| Exec | 调用 Permission 模块审批后执行 |

### 权限调用时机

Gateway 在以下场景调用 Permission 模块：
- 用户斜杠指令 `/exec`、`/git` 写操作 → evaluate()，Non-owner 高危指令默认 Deny

Gateway 自身的消息路由、Processor Chain 调度、Renderer 选择均不经过权限检查。工具调用的权限检查由 tools 模块触发，Gateway 不参与。

## 模块关系

### 上游（谁调用 Gateway）

| 模块 | 关系 |
|------|------|
| IM Adapter | 入站消息通过 Adapter 进入 Gateway 入站处理 |
| ChatServer | TCP 连接通过 Gateway 路由消息 |
| Session | LLM 响应以 ContentBlock[] 形式传入 Gateway 出站发送 |
| SlashDispatcher | SlashResult 由 Gateway 执行副作用并出站 |

### 下游（Gateway 调用谁）

| 模块 | 关系 |
|------|------|
| Processor Chain | 调度入站和出站处理器链 |
| Slash Command | 斜杠指令拦截后分派给 Dispatcher |
| Session Manager | 普通消息路由到 Session，由 SessionManager 处理消息 |
| Renderer | 选择对应平台 Renderer 完成出站渲染 |
| IM Adapter | 发送渲染后的消息 |
| Permission | 斜杠指令高危操作执行前校验 |

### 无关

- **System Prompt**（无调用关系）：Gateway 不参与 system prompt 构建或注入
- **LLM Provider**（无调用关系）：Gateway 不直接调用 LLM
- **Tools**（无调用关系）：Gateway 不注册工具、不执行工具调用

---

## 验证进度

> 草稿阶段追蹤表。每 session 验证一个方向，定稿后移除。

| # | 验证方向 | 进度 | 验证内容 |
|---|---------|------|---------|
| 1 | 入站路由 ←→ Processor Chain | ✅ | Gateway 描述的入站流程与 processor_chain/README.md 一致 — 2026-05-23 验证通过 |
| 2 | 出站发送 ←→ Renderer | ⬜ | Gateway 描述的出站流程与 renderer/README.md 一致 |
| 3 | 斜杠拦截 ←→ Slash | ⬜ | 斜杠拦截和 SlashResult 处理与 slash/README.md 一致 |
| 4 | Session 交互 ←→ Session | ⬜ | Gateway 调用 SessionManager 的方式与 session/README.md 一致 |
| 5 | Permission 调用 ←→ Permission | ⬜ | Gateway 调用 Permission 的场景与 permission/README.md 一致 |
