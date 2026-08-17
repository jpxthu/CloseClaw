# Gateway

## 概述

关联需求文档：[requirements/gateway.md](../../requirements/gateway.md)

Gateway 是消息路由中枢。它管理所有 IM 平台的插件，调度 Processor Chain 完成消息的出入站处理，做出路由决策（斜杠指令 vs 普通对话），并选择对应平台的 IM 插件完成出站消息的格式转换与发送。

Gateway 自身不含业务逻辑，通过编排下游模块完成消息流转。入站方向维护有界持久化消息队列缓冲高并发请求，出站方向根据交付模式协调 Processor Chain 执行时机。LLM 回复和斜杠指令回复统一经出站 Processor Chain 处理后发送。非文本错误回复经简化出站路径发送；系统通知（如"⏳ 正在排队..."、"正在恢复会话..."）由各业务模块（Session 等）经 Gateway 的通用系统通知接口发送，同样走简化出站路径。

## 架构

Gateway 由以下职责组成：

- **IM Adapter 管理**：注册和维护各平台插件（含 webhook 平台和 terminal 通道的 CLI）。入站方向将平台原始格式归一化为统一结构。
- **Processor Chain 调度**：按 priority 顺序调度入站和出站处理器链。入站链完成消息日志记录、session_key 计算和文本标准化。出站链按交付模式决定执行时机——批量模式一次性执行完整链；流式模式分增量阶段（逐 chunk 透传渲染，DslParser 零开销透传、跳过出站调试日志。流式开始前执行一次出站中间件链做 pre-flight 检查，被拒则终止流式并发送拒绝通知）、收尾阶段（执行 DSL 解析和出站调试日志，不重跑 VerbosityFilter。流式模式下 DSL 指令仅用于日志记录和出站历史写入，不产生渲染输出）和出错降级（流式进行中出错时终止流式会话，经简化出站路径追加错误提示，详见「出站路径」流式模式）。
- **路由决策**：根据消息前缀决定走向——以 `/` 开头则拦截分派给斜杠指令处理（其中 `/approve-once`、`/approve-whitelist`、`/deny` 在 Gateway 层硬拦截不进 SlashDispatcher），否则路由到 Session 进入 LLM 对话流程。普通消息路由前，Gateway 先根据配置定义的机器人→Agent 绑定确定对应的 Agent，将 agent_id 一并传给 SessionManager。
- **出站中间件**：渲染完成后、发送前，Gateway 按注册顺序链式执行中间件。内置审计中间件（记录敏感操作审计日志）和频率限制中间件（按 session 维度限频）。中间件不得修改消息内容，任一返回拒绝则消息不发送并记录告警。
- **IM Adapter 选择与渲染**：出站方向根据目标平台选择对应 IM Adapter，调用其渲染接口产出平台格式内容。渲染完成后、发送前，Gateway 执行中间件链，通过后调用 IM Adapter 的发送接口完成消息投递。渲染和发送为分离接口。
- **出站历史记录**：出站消息发送后，Gateway 将消息写入 session checkpoint 持久化存储，作为对话历史的出站部分。
- **系统通知接口**：Gateway 提供通用系统通知发送接口，供 Session 等模块发送纯文本系统通知（如"正在恢复会话..."、"⏳ 正在排队..."）。系统通知走简化出站路径（跳过 VerbosityFilter/DslParser/出站中间件）。通知内容与触发时机由调用方模块负责。
- **系统生命周期管理**：Gateway 参与优雅关闭——响应 ShutdownHandle 的 drain 计数（消息处理开始时递增、响应发送完成后递减），排空入站队列，等待进行中的流式会话完成后退出。关闭流程由 Daemon 的 ShutdownHandle 统一协调，Gateway 是被管理组件之一。

Gateway 维护以下运行时注册表：

- **Plugin Registry**：platform → IMPlugin 的映射
- **Processor Registry**：入站/出站处理器链，按 priority 排序
- **入站消息队列**：有界持久化缓冲队列，默认容量 256，可通过配置调整。满则拒收并回复用户，不支持动态调整。消息入队即持久化，处理完成后删除，重启时重放未完成消息（详见「消息队列与排队语义」）

**明确不做的职责**（详见下方无关表）：Bootstrap 加载与 System Prompt 构建、LLM 调用、工具注册与工具调用的直接执行。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [入站流程](inbound-flow.md) | 入站完整链路：IM Adapter 解析 → Processor Chain → Gateway 路由决策 |

### 模块分层和数据流

**入站**：

1. IM 平台 webhook 到达 Gateway，记录「webhook 到达」日志。
2. 消息进入入站消息队列（有界持久化缓冲，默认 256）。入队即持久化；队列满则拒收，经对应平台回复"服务繁忙，请稍后重试"并记录「满拒绝」日志。出队时记录「消息出队」日志。
3. IM Adapter 将平台格式解析为 NormalizedMessage。
4. Processor Chain 入站依次执行 RawLog（条件组件）→ SessionRouter → ContentNormalizer，产出 ProcessedMessage。
5. Gateway 做非文本检测：message_type 非 text（image/file/audio）→ 构造"暂不支持该消息类型"错误回复 ContentBlock[] → 简化出站（跳过 Verbosity/DslParser/中间件，经出站调试日志→渲染→发送），流程结束。
6. text 消息继续：Gateway 根据配置定义的机器人→Agent 绑定确定对应的 Agent，得到 agent_id。
7. Gateway 调用 SessionManager（传入 agent_id + session_key + 路由字段），SessionManager 内部提取稳定路由键做查找/创建，获得 session_id。
8. Gateway 路由决策：
   - `/` 开头 → 斜杠指令：
     - `/approve-once`、`/approve-whitelist`、`/deny` → Permission 审批流程（异步等待 Owner 审批）→ 通过后执行 → ContentBlock[] → 出站。
     - 其余斜杠 → SlashDispatcher → SlashResult → SideEffectContext 执行 → ContentBlock[]（进入出站）。
   - 普通消息 → Session → LLM → ContentBlock[]（LLM 响应，进入出站）。

**出站**（ContentBlock[] 来源：LLM 响应由 Session 产出，或斜杠指令回复由 SlashResult 变体产出。Gateway 按交付模式选择不同执行时序）：

**批量模式**：

1. ContentBlock[] → Processor Chain 出站（VerbosityFilter → DslParser → OutboundRawLog，一次性执行完整链）。Verbosity 过滤等级定义见 [slash 模块 verbose 指令](../slash/verbose.md)。
2. 产出 ProcessedMessage（含 content_blocks 与 dsl_result 元数据）。
3. Gateway 选择 IM Adapter → 一次渲染 → 中间件链（审计、频率限制，按注册顺序执行）→ 一次发送。
4. 出站历史记录持久化到 session checkpoint。

**流式模式**：

1. Pre-flight：增量阶段开始前执行出站中间件链（审计、频率限制）。被拒则终止流式并发送拒绝通知。
2. 增量阶段：ContentBlock[chunk₀] → VerbosityFilter → DslParser 透传 → IM Adapter 增量渲染 → 逐片发送，循环处理多轮 chunk。
3. 收尾阶段：ContentBlock[] 完整后执行 DslParser 完整解析 → OutboundRawLog（条件组件）。流式模式下 DSL 指令仅用于日志记录和出站历史写入，不产生渲染输出。随后出站历史记录持久化到 session checkpoint。
4. 出错降级：流式进行中出错（LLM 流中断或 IM 发送失败）时，Gateway 终止流式会话，经简化出站路径追加错误提示（明确标记回复不完整），出站历史记录已发送部分并写入错误事件标记。

Gateway 管理流式会话状态，跟踪当前流式进度并协调增量阶段、收尾阶段与出错降级的状态衔接。
渲染细节（行缓冲、块类型路由、平台格式转换）由 IM Adapter 内部负责，Gateway 不介入渲染逻辑。

关键交接：
- NormalizedMessage：IM Adapter 产出，Processor Chain 消费
- [ProcessedMessage](../common/shared-types.md#processedmessage)：Processor Chain 产出，Gateway 消费
- ContentBlock[]：LLM 响应 / SlashResult 变体产出，Processor Chain 出站消费
- RenderedOutput：Gateway 调用 IM Adapter 渲染产出（render 接口），渲染完成后 Gateway 执行中间件，通过后调用 IM Adapter 发送接口完成投递
- **SideEffectContext**：Gateway 构造，封装 Session 引用和回复通道。传给 [SlashResult](../common/shared-types.md#slashresult) 让各变体自行完成副作用，Gateway 不穷举变体。回复内容经出站 Processor Chain 处理后发送（详见 [Slash 模块](../slash/README.md)及 [出站链路](../processor_chain/outbound-chain.md)）

## 数据流

### 入站路径

Gateway 收到入站 webhook 后，消息先进入入站消息队列（有界持久化缓冲，详见下方「消息队列与排队语义」），再由 IM Adapter 解析后进入 Processor Chain。Processor Chain 入站产出 [ProcessedMessage](../common/shared-types.md#processedmessage) 后，Gateway 按以下路径处理：

- **非文本消息处理**：若消息的 message_type 非 text（image/file/audio），Gateway 直接构造"暂不支持该消息类型"的错误回复（ContentBlock[]），经简化出站路径发送。简化出站路径是 Gateway 层面的出站通道选择——错误回复为纯文本不含 DSL 指令且无需按 Session 过滤，因此跳过 VerbosityFilter/DslParser/中间件；若 `raw_log_dir` 已配置则执行 OutboundRawLog 写调试日志（简化路径下 OutboundRawLog 作为独立组件直接调用，不依赖 Processor Chain 调度），然后渲染发送。流程到此结束。

- **Session 解析**：Gateway 从 metadata 取出 session_key，并根据配置定义的机器人→Agent 绑定确定对应的 Agent，得到 agent_id。若 session_key 为空（SessionRouter 计算失败），Gateway 记录 warning 日志，仍通过消息路由字段（platform, sender_id, peer_id, account_id）传给 SessionManager 正常完成 session 查找/创建（详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md)）。session_key 是消息级追踪标识，不参与 session 路由——SessionManager 从路由字段中提取稳定路由键做查找。Gateway 将 agent_id 连同 session_key、路由字段一并传给 SessionManager，session 在 Agent 范围内隔离（详见 [Session 模块](../session/README.md)）。若 SessionManager 的 session 查找或创建操作失败（如存储异常），Gateway 向 User 回复错误提示，不进入后续 LLM 对话流程。

- **路由决策**：获得 session_id 后按消息内容路由：
  - **`/` 开头 → 斜杠指令**：先拦截 `/approve-once`、`/approve-whitelist`、`/deny`。
    - 非 Owner 调用 `/approve-once`、`/approve-whitelist`、`/deny` → Gateway 直接回复"权限不足：该指令仅限 Owner 使用"，不进入 Permission 模块和 SlashDispatcher。流程到此结束。
    - `/approve-once`、`/approve-whitelist`、`/deny`（Owner 调用）：经 ApprovalFlow 管理审批流转（详见 [Permission 模块审批工作流](../permission/approval-workflow.md)）。审批通过后执行对应操作，结果经出站链路发送。
    - 其余斜杠指令分派给 SlashDispatcher。Gateway 将 session_id 传给 SlashDispatcher 作为执行上下文（权限校验依赖）。消息不进入 LLM，不追加到对话历史。
    - Immediate 指令（如 `/stop`、`/status`、`/help` 等）→ 绕过 Session 忙碌队列立即执行。完整 Immediate 标记见 [Slash 模块 Handler 清单](../slash/README.md#handler-清单)。
    - 非 Immediate 指令 → 若 Session 正忙则进入 Session 忙碌队列（FIFO），Session 空闲后取出执行。入队时由 Session 生成"⏳ 正在排队..."提示语，经 Gateway 的通用系统通知接口发送。
  - **普通消息**：若 Session 正忙则进入 Session 忙碌队列；如果 Session 空闲，则直接进入 LLM 对话流程。若 Session 处于 archived 状态，由 SessionManager 触发 restore 流程，Session 生成"正在恢复会话..."提示语，经 Gateway 的通用系统通知接口发送。Session 就绪后进入 LLM 对话流程，返回 ContentBlock[] 进入出站链路。

> 斜杠指令的解析和 SlashResult 处理详见 [slash 模块](../slash/README.md)。Session 的创建、查找、归档、恢复详见 [Session 模块](../session/README.md)。审批流程详见 [Permission 模块审批工作流](../permission/approval-workflow.md)。

**Gateway 调试日志**：Gateway 在以下环节记录调试日志（不含原始消息 Payload——原始内容日志由 Processor Chain 的 RawLog 组件负责）：

- **入站消息到达与队列操作**：记录 webhook 到达、消息出队和队列满拒绝日志
- **路由决策结果**：记录识别结果（普通对话 / 斜杠指令 / 排队等待），含 session_id 和决策依据
- **中间件拦截**：中间件返回拒绝时记录告警日志，含拒绝原因和 session 标识

> 日志格式、级别、追踪标识、存储轮转和隐私脱敏遵循 [debug_log 框架](../debug_log/README.md)。Session 查找与生命周期事件日志由 [Session 模块](../session/README.md) 负责。出站渲染与平台 API 发送结果日志由 [IM Adapter 模块](../im_adapter/README.md) 负责。

### 出站路径

出站路径按交付模式分两种执行时序，但走同一组 Processor Chain 处理器和同一条 IM Adapter 渲染管线：

**批量模式**：LLM 返回完整 ContentBlock[] 后，Gateway 一次性送入 Processor Chain 出站链（VerbosityFilter → DslParser → OutboundRawLog），处理完毕后选择 IM Adapter 一次性渲染。渲染完成后由 Gateway 执行中间件链（审计、频率限制等），通过后的消息由 IM Adapter 发送。发送成功后 Gateway 将消息写入 session checkpoint 持久化存储（出站历史记录）。

**流式模式**：LLM 逐片产出 ContentBlock[] 增量。Gateway 分四个步骤调度：
1. **Pre-flight 中间件**：增量阶段开始前，Gateway 执行出站中间件链（审计、频率限制）。中间件基于 Session 元数据做预检——被拒则终止流式，Gateway 经简化出站路径发送拒绝通知（跳过中间件，避免同一中间件再次拒绝）；通过则进入增量阶段。
2. **增量阶段**：每个 chunk 经 VerbosityFilter 过滤后送入 DslParser（增量文本零开销透传，无 DSL 指令），跳过 OutboundRawLog（出站调试日志）。Gateway 交付 IM Adapter 增量渲染并逐片发送。
3. **收尾阶段**：全部 ContentBlock[] 到齐后，Gateway 执行 DslParser 完整解析 DSL 指令 → OutboundRawLog 写入出站调试日志。VerbosityFilter 已在增量阶段按 chunk 过滤，收尾阶段不重跑。流式模式下 DSL 指令仅用于日志记录和出站历史写入，不产生新渲染输出——流式回复中实际发送的内容在增量阶段已全部完成。最后 Gateway 将增量阶段 VerbosityFilter 过滤后的完整消息写入 session checkpoint 持久化存储。
4. **出错降级**：流式进行中出错（LLM 流中断或 IM 发送失败）时，Gateway 终止流式会话，经简化出站路径追加"回复中断"错误提示（明确标记本次回复不完整），出站历史记录已发送部分并写入错误事件标记。

Gateway 管理流式会话状态，跟踪当前流式进度、累积消息内容，确保增量阶段、收尾阶段与出错降级的状态连贯。

**系统通知路径**：Gateway 提供通用系统通知发送接口，供 Session 等模块发送纯文本系统通知（如"⏳ 正在排队..."、"正在恢复会话..."）。通知内容与触发时机由调用方模块负责，Gateway 仅提供发送通道。系统通知是纯文本消息，不含 DSL 指令且无需 Verbosity 过滤，走简化出站路径——跳过 VerbosityFilter/DslParser/出站中间件，若 `raw_log_dir` 已配置则经 OutboundRawLog 写调试日志后渲染发送。Gateway 自身也通过同一接口发送入站队列满的"服务繁忙"拒绝通知。

斜杠指令的回复统一经批量模式出站——SlashResult 变体通过 SideEffectContext 的回复通道产出回复内容，由 Gateway 送入出站 Processor Chain 处理，经 IM Adapter 渲染发送。这保证了斜杠指令回复与 LLM 回复使用统一的 Verbosity 过滤、DSL 解析和日志记录链路。

**出站日志的两种形态**：
- **出站调试日志（OutboundRawLog）**：Processor Chain 内 processor，将 ContentBlock[] 写入调试日志文件。仅在 `raw_log_dir` 配置时注册，用于开发和问题定位。日志格式、分级和脱敏遵循 [debug_log 框架](../debug_log/README.md)。
- **出站历史记录**：Gateway 在消息发送成功后写入 session checkpoint 持久化存储，记录字段包括 timestamp、session_id、platform、ContentBlock[]、dsl_result。非文本错误回复和系统通知走简化出站路径，不写 session checkpoint。

### 消息队列与排队语义

Gateway 涉及两层排队：

**第 1 层：入站消息队列**

- 位置：IM 平台 webhook 到达后、进入 Processor Chain 之前
- 性质：有界持久化缓冲队列，默认容量 256。消息入队即持久化（WAL 追加），完整处理链结束后标记完成并删除
- 满行为：拒绝新消息，Gateway 根据 webhook 来源平台选择对应 IM Adapter 在 2 秒内回复"服务繁忙，请稍后重试"
- 重启行为：重启时重放未标记完成的消息，未完成处理的消息不丢失（at-least-once）。消息重放时按消息身份去重，避免重复处理。优雅关闭时 Gateway 先停收新消息、排空已有队列后再退出
- 用户感知：正常负载下，文本消息到达 Gateway 后应在 1 秒内收到系统响应或排队提示。队列满时的"服务繁忙"拒绝本身是快速系统响应（另有 2 秒兜底约束），不视为例外；仅审批等待等真正异步等待的场景不适用此 1 秒响应约束
- 消费：IM Adapter 按 FIFO 从队列取消息解析，送入 Processor Chain 串行处理
- webhook 确认：Gateway 在消息入队并持久化成功后 ack webhook（返回 HTTP 200），不等待完整处理链结束。队列满拒绝的消息未确认（ack），由对应平台自动重发

**第 2 层：Session 忙碌队列**

- 位置：Gateway 路由决策后、进入 LLM 对话或非 Immediate 斜杠指令执行前
- 触发：Session 正忙（LLM 调用中或前台工具执行中）时新消息入队
- 性质：FIFO 待处理队列，Session 空闲后自动取出队首消息
- 通知：非 Immediate 消息（普通消息和非 Immediate 斜杠指令）入队时，由 Session 生成"⏳ 正在排队..."提示语，经 Gateway 的通用系统通知接口发送；Immediate 斜杠指令绕过此队列
- 详见 [Session 模块执行状态](../session/README.md)

1. 入站消息（高并发）进入入站消息队列（第 1 层）。
   - 有空闲槽位 → 进入 Processor Chain → 路由。
   - 队列满 → 拒绝 + 回复"服务繁忙，请稍后重试"。
2. 路由决策：
   - Immediate 指令 → 绕过 Session 队列，直接执行。
   - 其他 → 判断 Session 是否空闲：
     - 空闲 → 直接处理。
     - 正忙 → 进入 Session 忙碌队列（第 2 层），通知"⏳ 正在排队..."。Session 空闲后 FIFO 取出，再按 Session 状态分派：
       - Session archived → restore → 通知"正在恢复会话..." → Session 就绪后按原路由分派（LLM / SlashDispatcher）。
       - Session active → 按原路由分派（LLM / SlashDispatcher）。

### 斜杠指令副作用执行

SlashDispatcher 返回 [SlashResult](../common/shared-types.md#slashresult) 后，Gateway 构造 SideEffectContext（封装 Session 引用和回复通道）并触发 SlashResult 执行。各 SlashResult 变体在其执行逻辑中通过上下文完成对应的 session 操作。Gateway 不穷举变体，副作用逻辑内聚在 slash 模块。

SlashResult 的执行通过上下文的回复通道产出回复内容，Gateway 将回复送入出站 Processor Chain（VerbosityFilter → DslParser → OutboundRawLog）处理后由 IM Adapter 渲染发送。详见 [Slash 模块](../slash/README.md)。

### 权限调用时机

Gateway 在以下场景调用 Permission 模块：

1. **`/approve-once`、`/approve-whitelist`、`/deny`**：消息路由阶段硬拦截——不进 SlashDispatcher，直接在 Gateway 层调用 Permission 模块的审批流程验证（owner 专用）。审批通过后执行操作，结果经出站链路发送。
2. **其他斜杠指令高危操作**（`/exec`、`/git` 写操作）：在 SlashDispatcher 分派到 Handler、Handler 返回 SlashResult 后、执行前校验。SlashResult 变体自带「需权限校验」标记，Gateway 读取该标记决定是否调用 Permission——不穷举高危变体清单，新增高危指令只需在新变体上声明标记。Handler 仅做指令解析（无副作用），权限引擎拿到完整操作信息后评估——非 Owner 默认 Deny，但可通过白名单规则授予特定 Agent-User 组合的执行权（详见 [Permission 模块](../permission/README.md)）。

Gateway 自身的消息路由、Processor Chain 调度、IM Adapter 选择均不经过权限检查。工具调用的权限检查由 tools 模块触发，Gateway 不参与。

### 出站中间件

Gateway 在渲染完成后、发送前提供中间件拦截点。流式模式下中间件在增量阶段开始前执行一次（pre-flight），被拒则终止流式并发送拒绝通知。中间件按注册顺序链式执行：

- **接口**：输入渲染后的出站消息（流式 pre-flight 模式下输入为 Session 元数据），输出放行（透传）或拒绝（含拒绝原因）
- **执行契约**：中间件不得修改消息内容，任一中间件返回拒绝则消息不发送并记录告警日志
- **内置中间件**：
  - **审计中间件**：记录敏感操作（如 /exec 结果、文件读写）的出站审计日志
  - **频率限制中间件**：按 session 维度限制出站消息频率，超限时丢弃并记录告警

## 模块关系

### 上游（输入来源）

| 模块 | 关系 |
|------|------|
| IM Adapter | 入站消息通过插件进入 Gateway 入站处理 |
| Session | LLM 响应以 ContentBlock[] 形式传入 Gateway 出站发送 |
| Config | Gateway 读取机器人与 Agent 的绑定关系（机器人→Agent 绑定由配置定义），据此确定普通消息路由到的 Agent |

### 下游（Gateway 调用谁）

| 模块 | 关系 |
|------|------|
| Processor Chain | 调度入站和出站处理器链 |
| SlashDispatcher | 斜杠指令拦截后分派给 SlashDispatcher |
| SessionManager | 调用 SessionManager（传入 agent_id + session_key 和消息路由字段），由 SessionManager 内部提取稳定路由键进行 session 查找/创建。生命周期实现由 SessionManager 负责 |
| IM Adapter | 选择对应平台插件完成出站渲染与发送 |
| Permission | 斜杠指令高危操作执行前校验 |
| ApprovalFlow | `/approve-once`、`/approve-whitelist`、`/deny` 指令的审批流转管理（审批请求入队、Owner 通知、回调处理） |

### 共享类型

跨模块传递的共享数据结构定义在 [common 模块](../common/README.md)：

- [NormalizedMessage](../common/shared-types.md#normalizedmessage)：IM Adapter 产出，Processor Chain 消费
- [ProcessedMessage](../common/shared-types.md#processedmessage)：Processor Chain 产出，Gateway 消费
- [ContentBlock](../common/shared-types.md#contentblock)：LLM 响应 / SlashResult 变体产出，Processor Chain 出站消费
- [SlashResult](../common/shared-types.md#slashresult)：SlashDispatcher 产出，Gateway 消费
- [DslParseResult](../common/shared-types.md#dslparseresult)：DslParser 产出，IM Adapter 消费（渲染）和 Gateway 消费（出站历史记录写入）
- SideEffectContext：Gateway 构造的执行上下文，封装 Session 引用和回复通道。定义见 [common SlashResult](../common/shared-types.md#slashresult)。Gateway 构造后触发 [SlashResult](../common/shared-types.md#slashresult) 执行，各变体通过上下文完成副作用
- RenderedOutput：IM Adapter 渲染产出，Gateway 中间件消费（只读不修改），最终由 IM Adapter 发送接口投递

### 无关

- **Bootstrap**（无调用关系）：Gateway 不参与 Bootstrap 加载
- **System Prompt**（无调用关系）：Gateway 不参与 system prompt 构建或注入
- **LLM Provider**（无调用关系）：Gateway 不直接调用 LLM
- **Tools**（无调用关系）：Gateway 不注册工具、不执行工具调用
