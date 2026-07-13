# Gateway

## 概述

Gateway 是消息路由中枢。它管理所有 IM 平台的插件，调度 Processor Chain 完成消息的出入站处理，做出路由决策（斜杠指令 vs 普通对话），并选择对应平台的 IM 插件完成出站消息的格式转换与发送。

Gateway 自身不含业务逻辑，通过编排下游模块完成消息流转。入站方向维护有界消息队列缓冲高并发请求，出站方向统一经 Processor Chain 处理所有回复（含斜杠指令）。

## 架构

Gateway 由五个职责组成：

- **IM Adapter 管理**：注册和维护各平台插件，入站方向将平台原始格式归一化为统一结构。
- **Processor Chain 调度**：按 priority 顺序调度入站和出站处理器链。入站链完成消息归一化和日志记录，出站链完成 Verbosity 过滤、DSL 解析和出站日志。
- **路由决策**：根据消息前缀决定走向——以 `/` 开头则拦截分派给 SlashDispatcher（其中 Immediate 指令绕过 Session 队列立即执行），否则路由到 Session 进入 LLM 对话流程。
- **IM Adapter 选择与渲染**：出站方向根据目标平台选择对应 IM Adapter，IM Adapter 完成 ContentBlock[]（定义见 [common ContentBlock](../common/shared-types.md#contentblock)）到平台原生格式的渲染。渲染完成后、发送前，Gateway 可插入审计、频率限制等中间件拦截出站消息。发送由 IM Adapter 完成。
- **出站日志持久化**：出站消息发送后，Gateway 将消息写入 session checkpoint 持久化存储。

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
         [IM Adapter: 平台格式解析]
              ↓
         NormalizedMessage
              ↓
         [Processor Chain 入站: RawLog→SessionRouter→ContentNormalizer]
              ↓
         ProcessedMessage
              ↓
         [Gateway: 非文本检测] → message_type 非 text（image/file/audio）→ 构造错误回复 ContentBlock[] → 跳过 Verbosity/DslParser/中间件 → 出站日志 → 渲染发送
              ↓（text 消息，继续）
         [Gateway: 调用 SessionManager（传入 session_key + 路由字段），SessionManager 内部提取稳定路由键做查找/创建]
              ↓
         [Gateway: 路由决策]
              ├─ /approve, /deny → Permission 模块 → 异步等待 Owner 审批
              ├─ / 开头 → SlashDispatcher → SlashResult → ContentBlock[]（进入出站）
              └─ 普通   → Session → LLM
                                     ↓
                                ContentBlock[]（LLM 响应，进入出站）

出站（ContentBlock[] 来源：LLM 响应由 Session 产出，或斜杠指令回复由 SlashResult 变体产出。流式和非流式走同一条处理路径，仅在渲染阶段分叉）：

  ContentBlock[] → [Processor Chain 出站: VerbosityFilter → DslParser → OutboundRawLog]
                     （Verbosity 过滤等级定义见 [slash 模块 verbose 指令](../slash/verbose.md)）
                 → ProcessedMessage { content_blocks, metadata[dsl_result] }
                 → [Gateway: 选择 IM Adapter → IM Adapter 内部渲染]
                     ├─ 批量模式 → 一次性渲染
                     └─ 流式模式 → 增量渲染
                 → [中间件插入点] — Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件
                 → IM Adapter 发送：批量模式一次性发送 / 流式模式逐片发送

流式和非流式均经 Processor Chain。DslParser 对流式增量文本零开销透传（无 DSL 指令）。
流式渲染是 IM Adapter 内部的渲染模式选择，Gateway 不感知渲染是批量还是流式。
```

关键交接：
- NormalizedMessage：IM Adapter 产出，Processor Chain 消费
- [ProcessedMessage](../common/shared-types.md#processedmessage)：Processor Chain 产出，Gateway 消费
- ContentBlock[]：LLM 响应 / SlashResult 变体产出，Processor Chain 出站消费
- RenderedOutput：Gateway 调用 IM Adapter 渲染产出，由 IM Adapter 内部发送
- **SideEffectContext**：Gateway 构造，封装 Session 引用和回复通道。传给 [SlashResult](../common/shared-types.md#slashresult) 让各变体自行完成副作用，Gateway 不穷举变体。回复内容经出站 Processor Chain 处理后发送（详见 [Slash 模块](../slash/README.md)及 [出站链路](../processor_chain/outbound-chain.md)）

### 子功能索引

| 文档 | 内容 |
|------|------|
| [入站流程](inbound-flow.md) | 入站完整链路：IM Adapter 解析 → Processor Chain → Gateway 路由决策 |

## 数据流

### 入站路径

Gateway 收到入站 webhook 后，消息先进入入站消息队列（有界缓冲，详见下方「消息队列与排队语义」），再由 IM Adapter 解析后进入 Processor Chain。Processor Chain 入站产出 [ProcessedMessage](../common/shared-types.md#processedmessage) 后，Gateway 按以下路径处理：

- **非文本消息处理**：若消息的 message_type 非 text（image/file/audio），Gateway 直接构造"暂不支持该消息类型"的错误回复（ContentBlock[]），经简化出站路径发送（错误回复为纯文本不含 DSL 指令且无需按 Session 过滤，跳过 Verbosity/DslParser/中间件），经出站日志记录后由 IM Adapter 渲染发送。流程到此结束。

- **Session 解析**：Gateway 从 metadata 取出 session_key。若 session_key 为空（SessionRouter 计算失败），Gateway 记录 warning 日志，仍通过消息路由字段（platform, sender_id, peer_id, account_id）传给 SessionManager 正常完成 session 查找/创建（详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md)）。session_key 非空时连同路由信息一并传递。

- **路由决策**：获得 session_id 后按消息内容路由：
  - **`/` 开头 → 斜杠指令**：先拦截 `/approve`、`/deny`（owner 专用，经 Permission 模块审批流程验证，异步等待 owner 决策），其余分派给 SlashDispatcher。Gateway 将 session_id 传给 SlashDispatcher 作为执行上下文（权限校验依赖）。消息不进入 LLM，不追加到对话历史。
    - Immediate 指令（如 `/stop`、`/status`、`/help` 等）→ 绕过 Session 忙碌队列立即执行。完整 Immediate 标记见 [Slash 模块 Handler 清单](../slash/README.md#handler-清单)。
    - 非 Immediate 指令 → 若 Session 正忙则进入 Session 待处理队列（FIFO），Session 空闲后取出执行。入队时回复"⏳ 正在排队..."通知用户。
  - **普通消息**：若 Session 正忙则进入 Session 待处理队列；空闲则直接进入 LLM 对话流程。若 Session 处于 archived 状态，由 SessionManager 触发 restore 流程，Gateway 向用户发送"正在恢复会话..."通知。Session 就绪后进入 LLM 对话流程，返回 ContentBlock[] 进入出站链路。

> 斜杠指令的解析和 SlashResult 处理详见 [slash 模块](../slash/README.md)。Session 的创建、查找、归档、恢复详见 [Session 模块](../session/README.md)。

### 出站路径

出站路径中，Gateway 将 ContentBlock[] 送入 Processor Chain 出站链处理（VerbosityFilter → DslParser → OutboundRawLog），处理完毕后选择 IM Adapter 渲染。渲染完成后由 Gateway 执行中间件链（审计、频率限制等），通过后的消息由 IM Adapter 发送。

斜杠指令的回复同样经此出站路径——SlashResult 变体通过 SideEffectContext 的回复通道产出回复内容，由 Gateway 送入出站 Processor Chain 处理，经 IM Adapter 渲染发送。这保证了斜杠指令回复与 LLM 回复使用统一的 Verbosity 过滤、DSL 解析和日志记录链路。

### 消息队列与排队语义

Gateway 涉及两层排队：

**第 1 层：Gateway 入站队列**

- 位置：IM 平台 webhook 到达后、进入 Processor Chain 之前
- 性质：有界缓冲队列，不持久化
- 满行为：拒绝新消息，Gateway 通过 IM Adapter 回复"服务繁忙，请稍后重试"
- 重启行为：队列清空。非持久化队列中已入队但未处理的消息在重启时丢失。已到达 Gateway 但未返回响应的 webhook 由 IM 平台自动重试补偿。优雅关闭时 Gateway 应先停收新消息、排空已有队列后再退出，减少丢失范围
- 消费：IM Adapter 按 FIFO 从队列取消息解析，送入 Processor Chain 串行处理

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

SlashDispatcher 返回 [SlashResult](../common/shared-types.md#slashresult) 后，Gateway 构造 SideEffectContext（封装 Session 引用和回复通道）并触发 SlashResult 执行。各 SlashResult 变体在其执行逻辑中通过上下文完成对应的 session 操作。Gateway 不穷举变体，副作用逻辑内聚在 slash 模块。

SlashResult 的执行通过上下文的回复通道产出回复内容，Gateway 将回复送入出站 Processor Chain（VerbosityFilter → DslParser → OutboundRawLog）处理后由 IM Adapter 渲染发送。详见 [Slash 模块](../slash/README.md)。

### 权限调用时机

Gateway 在以下场景调用 Permission 模块：

1. **`/approve`、`/deny`**：消息路由阶段硬拦截——不进 SlashDispatcher，直接在 Gateway 层审批校验（owner 专用）。
2. **其他斜杠指令高危操作**（`/exec`、`/git` 写操作）：在 SlashDispatcher 分派到 Handler、Handler 返回 SlashResult 后、执行前校验。Handler 仅做指令解析（无副作用），权限引擎拿到完整操作信息后评估——非 Owner 默认 Deny，但可通过白名单规则授予特定 Agent-User 组合的执行权（详见 [Permission 模块](../permission/README.md)）。

Gateway 自身的消息路由、Processor Chain 调度、IM Adapter 选择均不经过权限检查。工具调用的权限检查由 tools 模块触发，Gateway 不参与。

## 模块关系

### 上游（谁调用 Gateway）

| 模块 | 关系 |
|------|------|
| IM Adapter | 入站消息通过插件进入 Gateway 入站处理 |
| Session | LLM 响应以 ContentBlock[] 形式传入 Gateway 出站发送 |

### 下游（Gateway 调用谁）

| 模块 | 关系 |
|------|------|
| Processor Chain | 调度入站和出站处理器链 |
| SlashDispatcher | 斜杠指令拦截后分派给 Dispatcher |
| SessionManager | 调用 SessionManager（传入 session_key 和消息路由字段），由 SessionManager 内部提取稳定路由键进行 session 查找/创建。生命周期实现由 SessionManager 负责 |
| IM Adapter | 选择对应平台插件完成出站渲染与发送 |
| Permission | 斜杠指令高危操作执行前校验 |

### 无关

- **System Prompt**（无调用关系）：Gateway 不参与 system prompt 构建或注入
- **LLM Provider**（无调用关系）：Gateway 不直接调用 LLM
- **Tools**（无调用关系）：Gateway 不注册工具、不执行工具调用


