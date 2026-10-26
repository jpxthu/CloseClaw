# 流式渲染

## 概述

流式渲染是 IM Adapter 模块的渲染子功能，负责在 LLM 流式输出时逐事件产出增量内容，使用户无需等待完整响应即可看到已生成内容。流式输入是统一流式事件 [StreamEvent](../common/shared-types.md#streamevent)，产出为本批增量 [StreamingOutput](../common/shared-types.md#streamingoutput)；流式输出一律为富格式消息，格式锁定规则见 [README 平台渲染选择](README.md#平台渲染选择)。批量路径的代码块处理见 [代码块渲染](code-render.md)，需求见 [im_adapter §F3/§F4](../../requirements/im_adapter.md)。

## 架构

**调用关系**：Gateway 驱动流式渲染——向插件投递 StreamEvent、按 200ms 周期触发超时检查、在流结束或出错时请求收尾刷新。插件持有渲染器实例与每进行中回复的独立渲染状态（缓冲、块状态、消息上下文不跨回复共享），渲染器只产出 StreamingOutput；Gateway 将每批 StreamingOutput 组装为 RenderedOutput 后调用插件的发送能力投递。渲染器不直接发送、不感知平台网络细节。

流式渲染器承担三项职责（职责划分，不约束内部代码组织）：

- 行缓冲：对 Text 块内容逐行积累，以句末标点或换行为边界切分输出单元
- 类型路由：按块类型选择渲染路径
- 增量输出：达到输出条件的单元作为本批内容交 Gateway 投递

**行缓冲规则**：

- 以句末标点（`。！？.!?\n`）为行边界，达到边界立即输出当前行；输出单元保留行尾字符（换行/句末标点）。
- 连续空行压缩为一个空行单元下发，用于保持段落间距。
- 缓冲区超过固定阈值（约 100 字符）时强制输出并清空；缓冲内容超过 200ms 未触发输出事件时由 Gateway 的超时检查强制输出。首行输出需在首个 Text 块到达后 200ms 内完成。
- 文本模式与代码模式：运行时按 ``` 围栏标记切换，文本模式按句末标点切分，代码模式累计至闭合围栏。代码块输出粒度由平台设定——IM 平台适配器采用整块模式（代码块内容与围栏行作为一个输出单元下发，闭合前不参与阈值/超时），terminal 渠道采用逐行模式。
- 代码块以单个 Text 块内的围栏为界；跨块围栏不识别为代码块。未闭合围栏在 Text 块 BlockEnd 刷新时按普通文本输出，保证批量与流式判定一致。
- Thinking/ToolUse/ToolResult 块不参与行缓冲，BlockEnd 到达后整块交付平台格式渲染器；流式中断时已累积的 Thinking 作为完整块交付。Image/Audio/File 不以流式事件形式出现（媒体能力暂缓，当前不出现）。
- Verbosity 过滤只作用于非 Text 块（在 BlockEnd 判定）；Text 增量不经过 Verbosity 过滤。

**中断处理**：Error 到达时停止产出文本增量，未达输出条件的半行文本丢弃、不补发；Gateway 在终止流前请求渲染器刷出已累积的 Thinking 并作为完整块交付，随后经简化出站路径发送「回复中断」错误提示（需求 [§F4](../../requirements/im_adapter.md)）。Session 层的 StreamingSink 是旁路通知通道（由平台插件的发送能力实现），不由本子功能推送。

流式渲染只处理文本与非文本块，不涉及任何交互控件。

## 数据流

1. Session 将 LLM 流式响应转发的 StreamEvent 事件流交给 Gateway。
2. Gateway 的 Processor Chain 增量阶段执行：VerbosityFilter 过滤（非 Text 块在 BlockEnd 判定）。
3. Gateway 将 StreamEvent 逐个投递给插件的流式渲染器。
4. 渲染器逐事件处理：
   - Text 块（BlockStart → BlockDelta... → BlockEnd）→ BlockDelta 追加到行缓冲 → 检测围栏切换文本/代码模式 → 达到边界、阈值或超时即产出本批输出单元；BlockEnd 到达时刷新该块残行
   - Thinking/ToolUse 块 → BlockDelta 累积，BlockEnd 到达后整块交付；Error 先于 BlockEnd 到达时，已累积的 Thinking 作为完整块交付
   - Image/Audio/File → 不以流式事件形式出现，批量路径交由平台格式渲染器处理（媒体能力暂缓）
   - Error → 停止文本增量，丢弃半行文本，交付已缓冲 Thinking（见「中断处理」）
5. MessageEnd → 刷新所有缓冲、输出剩余内容、清空块状态与行缓冲上下文。
6. Gateway 将本批 StreamingOutput 组装为 RenderedOutput 并调用插件发送能力投递——同一回复在平台侧表现为一条消息的增量更新；平台不支持增量更新时按批次追加（平台能力见各插件文档）。审计、频率限制等中间件在增量阶段开始前基于 Session 元数据执行一次 pre-flight，不逐片检查消息内容。
7. Session 按事件边界重组完整 ContentBlock[] 写入对话历史；Gateway 另维护已过滤的发送视图（完整 ContentBlock[] 的过滤后视图，而非行碎片视图）用于出站历史写入。渲染器不消费任何一份。
8. Gateway 收尾阶段执行 OutboundRawLog 写日志，不产生新渲染输出。

## 模块关系

- **上游**：Gateway（投递 StreamEvent、驱动超时检查与收尾刷新）
- **下游**：Gateway（消费 StreamingOutput 并组装 RenderedOutput，调用插件发送能力投递）
- **与 common 原语的关系**：平台无关的流式渲染原语（[common StreamingRenderer](../common/core-traits.md#streamingrenderer)，trait 定义 + 默认实现 + 行缓冲）位于 [common](../common/README.md)；im_adapter 与 cli 各适配器持有并委托调用，emit 归属见 [README 通用渲染能力](README.md#插件体系)；除 common 外任何 crate 不得把 common 项经自身公共路径再导出（见 [STANDARDS.md 禁止二次出口](../STANDARDS.md)）
- **与 Processor Chain 的关系**：Gateway 按交付模式协调链执行；流式出站走增量阶段（VerbosityFilter 过滤），完整链处理在收尾阶段由 Gateway 调度（见本文「数据流」）。批量模式一次性执行完整链后渲染，详见 [Gateway 文档](../gateway/README.md)
- **无关**：Session 层的 [StreamingSink](../common/core-traits.md#streamingsink)（旁路增量文本/完成/错误通知，由平台插件的发送能力实现、经 Session 驱动，不经本子功能；Completion/Error 提示由 Gateway 与 Session 负责）
- **所属**：IM Adapter 模块的渲染子功能（持有并委托 common 流式渲染原语）
