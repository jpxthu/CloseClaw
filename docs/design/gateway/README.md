# Gateway

## 概述

Gateway 是消息路由中枢。它管理所有 IM 平台的插件，调度 Processor Chain 完成消息的出入站处理，做出路由决策（斜杠指令 vs 普通对话），并选择对应平台的 IM 插件完成出站消息的格式转换与发送。

Gateway 自身不含业务逻辑，通过编排下游模块完成消息流转。入站方向维护有界消息队列缓冲高并发请求，出站方向统一经 Processor Chain 处理所有回复（含斜杠指令）。

## 架构

Gateway 由五个职责组成：

- **IM 插件管理**：注册和维护各平台插件。入站方向将平台原始格式归一化为统一结构，出站方向调用插件完成渲染和发送。
- **Processor Chain 调度**：按 priority 顺序调度入站和出站处理器链。入站链完成消息归一化和日志记录，出站链完成 DSL 解析和日志记录。
- **Verbosity 过滤**：出站方向 ContentBlock[] 进入 Processor Chain 之前，按当前 Session 的 Verbosity 等级过滤信息块（详见 [slash 模块 verbose 指令](../slash/verbose.md)）。
- **路由决策**：根据消息前缀决定走向——以 `/` 开头则拦截分派给 SlashDispatcher（其中 Immediate 指令绕过 Session 队列立即执行），否则路由到 Session 进入 LLM 对话流程。
- **IM 插件选择**：根据目标平台选择对应 IM 插件，插件内部完成 ContentBlock[] 到平台原生格式的渲染和发送。

Gateway 维护以下运行时注册表：

- **Plugin Registry**：platform → IMPlugin 的映射
- **Processor Registry**：入站/出站处理器链，按 priority 排序
- **入站消息队列**：有界缓冲队列，暂存 Gateway 来不及处理的入站消息

**明确不做的职责**（详见下方无关表）：Bootstrap 加载与 System Prompt 构建、LLM 调用、工具注册与工具调用的直接执行。

### 模块分层和数据流

```
入站：
  webhook → webhook → webhook → ...（高并发）
              ↓
         [入站消息队列]（有界缓冲，满则拒 + 回复"服务繁忙，请稍后重试"）
              ↓
         [IM 插件: 平台格式解析]
              ↓
         NormalizedMessage
              ↓
         [Processor Chain 入站: RawLog→SessionRouter→ContentNormalizer]
              ↓
         ProcessedMessage
              ↓
         [Gateway: SessionManager 把 session_key 解析为 session_id]
              ↓
         [Gateway: 路由决策]
              ├─ / 开头 → SlashDispatcher → SlashResult → ContentBlock[]（进入出站）
              └─ 普通   → Session → LLM
                                     ↓
                                ContentBlock[]（LLM 响应，进入出站）

出站（ContentBlock[] 来源：LLM 响应由 Session 产出，或斜杠指令回复由 SlashResult 变体产出；均经 Verbosity 过滤后进入 Processor Chain）：

  ContentBlock[] → [Verbosity 过滤]（详见 [slash 模块 verbose 指令](../slash/verbose.md)）
                 → [Processor Chain 出站: DslParser→RawLog]
                 → ProcessedMessage { content_blocks, metadata[dsl_result] }
                 → [Gateway: 选择 IM 插件 → 插件内部渲染]
                 → 插件直接发送到 IM 平台
```

关键交接：
- NormalizedMessage：IM Adapter 产出，Processor Chain 消费
- ProcessedMessage：Processor Chain 产出，Gateway 消费
- ContentBlock[]：LLM 响应 / SlashResult 变体产出，Processor Chain 出站消费
- RenderedOutput：Gateway 调用 IM 插件渲染产出，由插件内部 Adapter 发送
- **SideEffectContext**：Gateway 构造，封装 Session 引用和回复通道。传给 SlashResult.execute() 让各变体自行完成副作用，Gateway 不穷举变体。回复内容经出站 Processor Chain 发送（详见 [Slash 模块](../slash/README.md)）

### 子功能索引

| 文档 | 内容 |
|------|------|
| [入站流程](inbound-flow.md) | 入站完整链路：IM 插件解析 → Processor Chain → Gateway 路由决策 |

## 数据流

### 入站路径

Gateway 收到 Processor Chain 入站产出的 ProcessedMessage 后，按以下路径处理：

- **消息入队**：高并发入站时消息先进入有界缓冲队列。队列满时拒绝新消息并回复"服务繁忙，请稍后重试"。IM 插件按 FIFO 从队列取消息解析，送入 Processor Chain 串行处理。Gateway 重启时队列清空（消息由 IM 平台 webhook 重试补偿）。

- **Session 解析**：Gateway 从 metadata 取出 session_key。若 session_key 为空（SessionRouter 计算失败），Gateway 回复"会话路由失败，请重试"。非空时调用 SessionManager 把 session_key 解析为 session_id。

- **路由决策**：获得 session_id 后按消息内容路由：
  - **`/` 开头 → 斜杠指令**：先拦截 `/approve`、`/deny`（owner 专用，经 Permission 模块审批流程验证，异步等待 owner 决策），其余分派给 SlashDispatcher。Gateway 将 session_id 传给 SlashDispatcher 作为执行上下文（权限校验依赖）。消息不进入 LLM，不追加到对话历史。
    - Immediate 指令（如 `/stop`、`/status`、`/help` 等）→ 绕过 Session 忙碌队列立即执行。完整 Immediate 标记见 [Slash 模块 Handler 清单](../slash/README.md#handler-清单)。
    - 非 Immediate 指令 → 若 Session 正忙则进入 Session 待处理队列（FIFO），Session 空闲后取出执行。入队时回复"⏳ 正在排队..."通知用户。
  - **普通消息**：若 Session 正忙则进入 Session 待处理队列；空闲则直接进入 LLM 对话流程。若 Session 处于 archived 状态，由 SessionManager 触发 restore 流程，Gateway 向用户发送"正在恢复会话..."通知。Session 就绪后进入 LLM 对话流程，返回 ContentBlock[] 进入出站链路。

> 斜杠指令的解析和 SlashResult 处理详见 [slash 模块](../slash/README.md)。Session 的创建、查找、归档、恢复详见 [Session 模块](../session/README.md)。

### 出站路径

Gateway 收到 ContentBlock[] 后（来源：LLM 响应，由 Session 产出；或 SlashResult 变体产出），经 Verbosity 过滤后调度出站 Processor Chain 处理得到 ProcessedMessage，然后根据目标平台选择 IM 插件，插件内部渲染后直接发送。

斜杠指令的回复同样经此出站路径——SlashResult 变体通过 SideEffectContext 的回复通道产出回复内容，由 Gateway 送入出站 Processor Chain 处理（DslParser → RawLog），再经 IM 插件渲染发送。这保证了斜杠指令回复与 LLM 回复使用统一的日志记录和 DSL 解析链路。

### 消息队列与排队语义

Gateway 涉及两层排队：

**第 1 层：Gateway 入站队列**

- 位置：IM 平台 webhook 到达后、进入 Processor Chain 之前
- 性质：有界缓冲队列，不持久化
- 满行为：拒绝新消息，Gateway 通过 IM 插件回复"服务繁忙，请稍后重试"
- 重启行为：队列清空。Gateway 重启期间 IM 平台 webhook 不可达，恢复后由 IM 平台重试未送达的 webhook
- 消费：IM 插件按 FIFO 从队列取消息解析，送入 Processor Chain 串行处理

**第 2 层：Session 忙碌队列**

- 位置：Gateway 路由决策后、进入 LLM 对话前
- 触发：Session 正忙（LLM 运行中、工具执行中）时新消息入队
- 性质：FIFO 待处理队列，Session 空闲后自动取出队首消息
- 通知：普通消息入队时回复"⏳ 正在排队..."；Immediate 斜杠指令绕过此队列
- 详见 [Session 模块执行状态](../session/README.md)

```
入站消息（高并发）
  ↓
[Gateway 入站队列]（第 1 层）
  ├─ 有空闲槽位 → 进入 Processor Chain → 路由
  └─ 队列满 → 拒绝 + 回复"服务繁忙，请稍后重试"
  ↓
路由决策
  ├─ Immediate 指令 → 绕过 Session 队列，直接执行
  └─ 其他 → Session 空闲？
            ├─ 空闲 → 直接处理
            └─ 正忙 → [Session 忙碌队列]（第 2 层）→ 通知"排队中"
                           ↓
                       Session 空闲后 FIFO 取出
                         ↓
                    按原路由分派（LLM / SlashDispatcher）
```

### 斜杠指令副作用执行

SlashDispatcher 返回 SlashResult 后，Gateway 构造 SideEffectContext（封装 Session 引用和回复通道）并调用 SlashResult.execute()。各 SlashResult 变体在 execute() 内通过 ctx 完成对应的 session 操作。Gateway 不穷举变体，副作用逻辑内聚在 slash 模块。

SlashResult.execute() 通过 SideEffectContext 的回复通道产出回复内容，Gateway 将回复送入出站 Processor Chain（DslParser → RawLog）统一处理后由 IM 插件渲染发送。详见 [Slash 模块](../slash/README.md)。

### 权限调用时机

Gateway 在以下场景调用 Permission 模块（在 SlashDispatcher 分派到 Handler、Handler 返回 SlashResult 后，SlashResult.execute() 执行前校验）：
- 用户斜杠指令 `/exec`、`/git` 写操作 → 调用权限评估，Non-owner 高危指令默认 Deny

Gateway 自身的消息路由、Processor Chain 调度、IM 插件选择均不经过权限检查。工具调用的权限检查由 tools 模块触发，Gateway 不参与。

## 模块关系

### 上游（谁调用 Gateway）

| 模块 | 关系 |
|------|------|
| IM Adapter | 入站消息通过插件进入 Gateway 入站处理 |
| Session | LLM 响应以 ContentBlock[] 形式传入 Gateway 出站发送 |
| SlashDispatcher | SlashResult 由 Gateway 调用 execute() 执行副作用，回复经出站链发送 |

### 下游（Gateway 调用谁）

| 模块 | 关系 |
|------|------|
| Processor Chain | 调度入站和出站处理器链 |
| SlashDispatcher | 斜杠指令拦截后分派给 Dispatcher |
| SessionManager | 调用 SessionManager 把 session_key 解析为 session_id，编排 Session 的创建、查找、恢复。生命周期实现由 SessionManager 负责 |
| IM Adapter | 选择对应平台插件完成出站渲染与发送 |
| Permission | 斜杠指令高危操作执行前校验 |

### 无关

- **System Prompt**（无调用关系）：Gateway 不参与 system prompt 构建或注入
- **LLM Provider**（无调用关系）：Gateway 不直接调用 LLM
- **Tools**（无调用关系）：Gateway 不注册工具、不执行工具调用


