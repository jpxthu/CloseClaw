# 流式渲染

## 概述

流式渲染是 IM Adapter 模块的通用渲染子功能，负责在 LLM 流式输出时逐事件渲染增量内容。流式输入是统一流式事件 [StreamEvent](../common/shared-types.md#streamevent)（ContentBlock 的流式形态），用户无需等待完整响应即可看到输出内容。该能力以通用组件形式提供——各平台插件组合持有流式渲染器实例，在渲染时委托调用，平台可按需覆盖实现差异化渲染逻辑。

## 架构

流式渲染器在 Gateway 的增量阶段被调用，输入为 StreamEvent 事件流（经 VerbosityFilter 按块边界过滤 + DslParser 透传）。完整链处理（DslParser 解析 DSL 指令 + 出站日志）由 Gateway 在收尾阶段调度执行，不属于流式渲染器。

流式渲染器承担三项职责（职责划分，不约束内部代码组织）：

- 行缓冲：对 Text 块内容逐行积累，以句末标点或换行为边界切分输出单元
- 类型路由：按块类型选择渲染路径
- 增量输出：完整输出单元立即通过 IMPlugin 发送

流式渲染器逐事件消费 StreamEvent（增量载荷结构 [ContentDelta](../common/shared-types.md#contentdelta)，本批产出结构见 [common StreamingOutput](../common/shared-types.md#streamingoutput)）——BlockDelta 到达即驱动 Text 块逐缓冲行输出（块未结束即可输出）；Thinking/Tool 块等待对应 BlockEnd 全块就绪后一次交付平台格式渲染器。交互式 UI 元素（按钮、选择器等）通过工具调用结果由 Gateway 直接处理，不属于流式渲染器职责范围。

**行缓冲规则**：

- 以句末标点（`。！？.!?\n`）为行边界，达到边界立即输出当前行
- 代码块内按换行符输出，不做句末标点等待
- 目标平台需要完整代码块才能正确渲染语法高亮时，以完整代码块为单位输出（代码块结束后一次性发送），不逐行输出代码块内容；此时代码块内容不参与 100 字符阈值和 200ms 超时的强制输出
- 缓冲区超过固定阈值（约 100 字符）时强制输出并清空缓冲区；缓冲内容超过 200ms 未触发输出事件时强制输出。首行输出需在首个 Text 块到达后 200ms 内完成——若缓冲内容在 200ms 内未达输出条件，强制输出当前缓冲内容
- Thinking/Tool 块不参与流式行缓冲，累积完整内容后一次交付平台格式渲染器；Image/Audio/File 不以流式事件形式出现（LLM 流式不产出媒体块），非流式路径中直接交由平台格式渲染器处理
- 代码/文本模式状态：检测 ``` 边界标记切换，用于决定行边界判定规则（代码模式按换行，文本模式按句末标点）

## 数据流

1. StreamEvent 事件流到达 Gateway（Session 转发的 LLM 流式响应）
2. Gateway 的 Processor Chain 增量阶段依次执行：VerbosityFilter 过滤（按块边界）→ DslParser 透传（零开销）→ 跳过 OutboundRawLog（出站调试日志，仅在 raw_log_dir 配置时注册）
3. Gateway 交付 StreamEvent 给 IMPlugin 流式渲染器
4. 流式渲染器逐事件消费，按事件类型处理：
   - Text 块（BlockStart → BlockDelta... → BlockEnd）→ BlockDelta 到达即追加文本到行缓冲区 → 检测代码块边界标记（```）切换代码/文本模式 → 检测句末标点或换行（文本模式）、换行（代码模式）或完整代码块结束（平台需要完整代码块时）。完整输出单元立即渲染输出，不完整则继续缓冲，缓冲区超过阈值（约 100 字符）或 200ms 超时则强制输出——不等待 BlockEnd
   - Thinking/Tool 块（BlockStart → BlockDelta... → BlockEnd）→ BlockDelta 累积内容，BlockEnd 到达即全块就绪，一次交付平台格式渲染器（如飞书的折叠推理区、工具操作卡片）
   - Image/Audio/File 块 → 不以流式事件形式出现（LLM 流式不产出媒体块），非流式路径中直接交由平台格式渲染器处理（图片内容的上下文/引用区分见 [im_adapter media-store](media-store.md)）
   - Error → 不产生增量输出，流错误的统一降级处理由 Gateway 负责（详见 [Gateway 出站流程](../gateway/outbound-flow.md)）
5. MessageEnd → 刷新所有缓冲 → 输出剩余内容 → 清空块状态和行缓冲上下文
6. 增量输出通过 IMPlugin 发送到 IM 平台（流式模式下 Gateway 的审计、频率限制等中间件在增量阶段开始前执行一次 pre-flight 检查，非逐片插入，详见 [Gateway 出站中间件](../gateway/outbound-flow.md)）
7. 消息级完整 ContentBlock[] 到齐（由 Gateway 按 BlockEnd 边界组装）
8. Gateway 的 Processor Chain 收尾阶段执行：DslParser 完整解析 DSL 指令 → OutboundRawLog 写入出站日志（此阶段不产生新渲染输出）

## 模块关系

- **上游**：Gateway（交付经 Processor Chain 处理后的 [StreamEvent](../common/shared-types.md#streamevent) 事件流给 IMPlugin，IMPlugin 内部触发流式渲染）
- **下游**：IMPlugin（接收增量渲染输出并通过 Adapter 发送到 IM 平台）
- **内部组件**：流式渲染器是 IM Adapter 的通用组件，由各平台插件组合持有并委托调用。平台可覆盖实现差异化渲染逻辑
- **与 Processor Chain 的关系**：Gateway 按交付模式协调链执行。流式出站走增量阶段——StreamEvent 事件流经 VerbosityFilter 过滤、DslParser 透传后进入流式渲染。完整链处理（DslParser 解析 DSL 指令 + 出站日志）在流式渲染完成后由 Gateway 在收尾阶段调度。批量模式一次性执行完整链后渲染，详见 [Gateway 文档](../gateway/README.md)
- **所属**：IM Adapter 模块的通用子功能
