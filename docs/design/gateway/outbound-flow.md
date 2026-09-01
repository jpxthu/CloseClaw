# 出站流程

## 概述

出站流程处理从回复内容产生到 IM 平台投递的完整链路。ContentBlock[]（来源：LLM 响应或斜杠指令回复）经出站 Processor Chain 处理后，由 Gateway 协调 IM Adapter 完成渲染和发送。Gateway 按交付模式（批量/流式）决定链的执行时序；错误/提示通知、降级提示和系统通知走简化出站路径；发送成功后写出站历史记录。

## 架构

### 执行模式

Gateway 按交付模式分两种执行时序调度同一条出站 Processor Chain 和同一条 IM Adapter 渲染管线：

**批量模式**：ContentBlock[] 完整到齐后，Gateway 一次性送入出站链（VerbosityFilter → DslParser → OutboundRawLog），处理完毕后选择 IM Adapter 一次性渲染。渲染完成后由 Gateway 执行中间件链（审计、频率限制等），通过后的消息由 IM Adapter 发送。发送成功后 Gateway 将消息写入 session checkpoint 持久化存储（出站历史记录）。斜杠指令的回复统一走批量模式——SlashResult 变体通过 SideEffectContext 的回复通道产出回复内容，由 Gateway 送入出站 Processor Chain 处理后渲染发送，保证斜杠指令回复与 LLM 回复使用统一的 Verbosity 过滤、DSL 解析和日志记录链路。

**批量出错降级**：出站链处理器异常按链级容错策略处理（VerbosityFilter 失败等同不过滤、DslParser 失败原样透传、OutboundRawLog 失败跳过日志，详见 [processor_chain 出站链路](../processor_chain/outbound-chain.md)），不阻塞发送。渲染或发送失败时消息未送达——批量模式一次性渲染发送，不存在部分送达，天然满足「不呈现不完整内容」；Gateway 经简化出站路径向 User 发送「回复发送失败」提示并记录告警日志，不自动重试（避免平台限流场景下重复发送）。

**流式模式**：LLM 逐事件产出 [StreamEvent](../common/shared-types.md#streamevent) 流式事件。Gateway 分四个阶段调度：

1. **Pre-flight 中间件**：增量阶段开始前，Gateway 执行出站中间件链（审计、频率限制）。中间件基于 Session 元数据做预检——被拒则终止流式，Gateway 经简化出站路径发送拒绝通知（跳过中间件，避免同一中间件再次拒绝）；通过则进入增量阶段。
2. **增量阶段**：LLM 流式响应以 [StreamEvent](../common/shared-types.md#streamevent) 事件流逐事件传递——经 VerbosityFilter 按块边界过滤后送入 DslParser（零开销透传，无 DSL 指令），跳过 OutboundRawLog（出站调试日志）。Gateway 交付 IM Adapter 流式渲染器逐事件增量渲染并逐片发送。
3. **收尾阶段**：全部 ContentBlock[] 到齐后，Gateway 执行 DslParser 完整解析 DSL 指令 → OutboundRawLog 写入出站调试日志。VerbosityFilter 已在增量阶段按块边界过滤，收尾阶段不重跑。流式模式下 DSL 指令仅用于日志记录和出站历史写入，不产生新渲染输出——流式回复中实际发送的内容在增量阶段已全部完成。最后 Gateway 将增量阶段 VerbosityFilter 过滤后的完整消息写入 session checkpoint 持久化存储。
4. **出错降级**：流式进行中出错（LLM 流中断或 IM 发送失败）时，Gateway 终止流式会话，经简化出站路径追加"回复中断"错误提示（明确标记本次回复不完整），出站历史记录已发送部分并写入错误事件标记。

Gateway 管理流式会话状态，跟踪当前流式进度、累积消息内容，确保增量阶段、收尾阶段与出错降级的状态连贯。

### 简化出站路径

Gateway 层面的出站通道选择。适用于以下纯文本、无 DSL 指令、无需按 Session 过滤的面向 User 提示（均不进入 LLM 对话链）：

- **媒体不可得提示**：入站路由时发现含媒体消息且媒体不可得（unavailable_media 非空：下载失败或超出大小上限）→ 提示「该消息内容无法获取」（形态规则见 [im_adapter media-store](../im_adapter/media-store.md)）
- **系统通知**：Session 等业务模块经 Gateway 通用系统通知接口发送的纯文本提示（如"⏳ 正在排队..."、"正在恢复会话..."）。通知内容与触发时机由调用方模块负责，Gateway 仅提供发送通道
- **流式降级提示**：pre-flight 拒绝通知、流式中断的"回复中断"提示
- **批量发送失败提示**：批量模式渲染/发送失败时的"回复发送失败"提示

简化路径跳过 VerbosityFilter / DslParser / 出站中间件，若 `raw_log_dir` 已配置则经 OutboundRawLog 写调试日志（作为独立组件直接调用，不依赖 Processor Chain 调度），然后渲染发送。简化路径消息均不写 session checkpoint（不属于对话历史）。除上述分类外，权限不足提示、Session 查找/创建失败提示等其他所有面向 User 的纯文本提示同样经此路径发送、不写出站历史——凡是不进入 LLM 对话链的 User 可见提示，均走简化出站路径而非完整出站链（见「系统通知接口」节的边界说明）。

### 系统通知接口

Gateway 提供通用系统通知发送接口，供 Session 等模块发送纯文本系统通知。系统通知是**面向 User 的提示**，经简化出站路径**投递到 IM 平台**，不注入 LLM 上下文、不写入 session 对话历史（与出站历史记录的关系见「出站历史记录」节）。系统通知是纯文本消息，不含 DSL 指令且无需 Verbosity 过滤。Gateway 自身也通过同一接口发送入站队列满的"服务繁忙"拒绝通知。

**边界（不做什么）**：系统通知仅指经本接口**投递到 IM** 的用户可见提示。别名为"通知"、但实际**注入对话历史、供 LLM 上下文处理**的系统级消息（如子 Session 超时/预警通知，经统一消息队列注入对话流）**不属于系统通知**——那属于消息注入机制，不走简化出站路径、也不经本接口，见 [session 统一消息队列](../session/session-execution.md#统一消息队列)。

### 出站历史记录

出站消息发送成功后，Gateway 将消息写入 session checkpoint 持久化存储，记录字段包括 timestamp、session_id、platform、ContentBlock[]、dsl_result。

**定位**：出站历史是**用户可见内容的交付记录**——记录用户实际收到什么（Verbosity 过滤后的内容与 DSL 解析结果）。与 Session 对话历史（messages[]，LLM 上下文，含完整 Thinking 块）用途不同：后者服务于上下文恢复，会随 compaction 演化；前者是交付审计，不随 compaction 改变。二者并存、各司其职。

### 出站中间件

Gateway 在渲染完成后、发送前提供中间件拦截点。流式模式下中间件在增量阶段开始前执行一次（pre-flight），被拒则终止流式并发送拒绝通知。中间件按注册顺序链式执行：

- **接口**：输入渲染后的出站消息（流式 pre-flight 模式下输入为 Session 元数据），输出放行（透传）或拒绝（含拒绝原因）
- **执行契约**：中间件不得修改消息内容，任一中间件返回拒绝则消息不发送并记录告警日志
- **内置中间件**：
  - **审计中间件**：记录敏感操作（如 /exec 结果、文件读写）的出站审计日志
  - **频率限制中间件**：按 session 维度限制出站消息频率，超限时丢弃并记录告警

### 出站日志的两种形态

- **出站调试日志（OutboundRawLog）**：Processor Chain 内 processor，将 ContentBlock[] 写入调试日志文件。仅在 `raw_log_dir` 配置时注册，用于开发和问题定位。日志格式、分级和脱敏遵循 [debug_log 框架](../debug_log/README.md)。
- **出站历史记录**：见上方「出站历史记录」节。流式收尾阶段与批量模式均先写调试日志再写出站历史（或错误事件标记）。

## 数据流

### 批量模式

1. ContentBlock[]（LLM 响应 / SlashResult 变体回复）进入出站 Processor Chain（VerbosityFilter → DslParser → OutboundRawLog，一次性执行完整链），产出 ProcessedMessage（content_blocks + dsl_result 元数据）。
2. Gateway 选择目标平台 IM Adapter 渲染（RenderedOutput）。
3. 中间件链执行（审计、频率限制，按注册顺序），通过后由 IM Adapter 发送。
4. 发送成功 → 出站历史记录写入 session checkpoint。渲染或发送失败 → 经简化出站路径发送「回复发送失败」提示并记录告警日志，不自动重试、不写出站历史，流程结束。

### 流式模式

1. Pre-flight：增量阶段开始前执行中间件链（基于 Session 元数据预检）。被拒 → 终止流式，经简化出站路径发送拒绝通知，流程结束。
2. 增量阶段：StreamEvent 事件 → VerbosityFilter（按块边界过滤）→ DslParser（零开销透传）→ IM Adapter 流式渲染器（逐事件增量渲染）→ 逐片发送。
3. 增量阶段结束后二选一：
   - 正常收尾（LLM 流完整）：完整 ContentBlock[] → DslParser 完整解析 → OutboundRawLog → 出站历史记录写入 session checkpoint。
   - 出错降级（LLM 流中断或 IM 发送失败）：终止流式 → 简化出站路径追加"回复中断"提示 → 出站历史记录已发送部分并写入错误事件标记。

### 简化出站路径

1. 纯文本内容（媒体不可得提示 / 系统通知 / 降级提示——流式中断与批量发送失败）。
2. OutboundRawLog（仅 `raw_log_dir` 已配置时，独立组件直调）。
3. IM Adapter 渲染 → 发送（不经 VerbosityFilter / DslParser / 中间件）。

**关键判断点**：

- **交付模式**：ContentBlock[] 完整到齐 → 批量模式；LLM 以 [StreamEvent](../common/shared-types.md#streamevent) 事件流逐事件产出 → 流式模式
- **通道选择**：对话回复（含斜杠指令回复）→ 完整出站链；媒体不可得提示 / 纯文本错误/提示回复 / 系统通知 / 降级提示（流式中断、批量发送失败）→ 简化出站路径
- **中间件拦截**：批量模式在渲染后发送前执行；流式模式前置为 pre-flight（基于 Session 元数据），避免增量发送后无法撤回
- **出错降级**：批量模式渲染/发送失败 → 简化路径发「回复发送失败」提示，不重试；流式模式增量中断 → 简化路径追加「回复中断」提示，出站历史记录已发送部分并标记错误事件
- **出站历史**：批量模式发送成功后写入；流式模式收尾阶段写入；简化路径不写；流式中断写入已发送部分并标记错误事件

## 模块关系

- **上游**：Session（LLM 响应的 ContentBlock[]）、SlashDispatcher（斜杠指令回复的 ContentBlock[]，经 SideEffectContext 回复通道）、业务模块（系统通知，经 Gateway 通用系统通知接口）
- **下游**：Processor Chain 出站（调度链执行）、IM Adapter（渲染 + 发送，两步分离，中间件在两步之间插入）
- **相关**：Session checkpoint 持久化（出站历史记录写入，交付审计定位见「出站历史记录」节）
- **无关**：Permission（出站流程不触发权限检查；斜杠指令的权限校验在入站路由阶段完成，见 [Gateway README](README.md#权限调用时机)）
