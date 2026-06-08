# 出站链路

## 概述

出站 Processor 链在 LLM 生成响应后运行。它从 Session 中读取 UnifiedResponse（ContentBlock[]），解析 DSL 指令，然后将处理结果交付给 IM Adapter 模块渲染。

出站链的职责仅限于**内容变换**（提取 DSL 指令、记录日志），不负责展示格式的生成。渲染由 IM Adapter 模块完成。

## 架构

出站链由两个 Processor 组成。渲染由 IM Adapter 模块完成，各平台有各自的 IM 插件（如飞书插件包含飞书 Adapter + 飞书 Renderer），Gateway 根据目标平台选择对应插件：

```
Session 消息（ContentBlock[]）
  ↓
Processor 链（出站，按 priority 升序执行）
  ├── DslParser（priority 10）
  │     → 遍历 ContentBlock[] 中的 Text 块
  │     → 匹配并解析 DSL 指令行（::button[...] 等）
  │     → 从 Text 块中剥离 DSL 行
  │     → 解析结果写入 metadata
  │     → Thinking / ToolUse / ToolResult 块直接透传
  │
  └── RawLogProcessor（priority 20）
        → 出站消息写入日志
  ↓
ProcessedMessage { content_blocks, metadata }
  ↓
IM Adapter 模块
  → 接收 ContentBlock[] + DslParseResult
  → 按块类型选择渲染策略
  → 输出平台原生格式并发送

IM Adapter 不在 Processor 链内：
- 渲染和发送是终结操作，输出后不再有下一步传递给其他 Processor
- 渲染需要携带 msg_type 路由信息（text / interactive），这和链的"变换传递"语义不符
- 各平台 IM 插件由 Gateway 根据目标平台选择

## 数据流

```
LLM 输出 UnifiedResponse（含 ContentBlock[]）
  → Session 写入 messages[]
    → Gateway 从 Session 读取 ContentBlock[]
      → 构造 ProcessedMessage，启动出站 Processor 链
        → DslParser.process(ctx)遍历 ContentBlock[]：
              ├── Text 块 → 逐行扫描 DSL → 解析 → 剥离 DSL 行
              ├── Thinking 块 → 透传
              ├── ToolUse 块 → 透传
              └── ToolResult 块 → 透传
            输出：更新的 ContentBlock[] + metadata["dsl_result"]
        → RawLogProcessor.process(ctx)
            输出：日志记录，内容不变
      → 链输出 ProcessedMessage
        → Gateway 选择目标平台 IM 插件
          → 插件内部渲染(content_blocks, dsl_result)：
              ├── Text 块 → 平台文本 / 富文本格式
              ├── Thinking 块 → 平台折叠内容
              ├── ToolUse 块 → 平台工具调用展示
              └── ToolResult 块 → 平台工具结果展示
          → 输出平台原生格式并发送
        → 根据 (peer_id, thread_id) 发送到对应会话/话题
```

关键判断点：
- DslParser 仅处理 ContentBlock::Text 类型，其他块透传
- 无 DSL 指令时 DslParser 输出与输入一致（零开销透传）
- 无目标平台或平台不支持时，回退到纯文本输出

## 模块关系

- **上游**：Session（LLM 对话产出的 ContentBlock[]，经 Gateway 传递进入链，属数据流上游依赖）、SlashDispatcher（斜杠指令回复的 ContentBlock[]）
- **下游**：[IM Adapter](../im_adapter/README.md) 模块（消费 ContentBlock[] + DslParseResult，渲染为平台格式并发送）
- **链内**：
  - DslParser — 解析 DSL 指令，为渲染提供交互数据
  - RawLogProcessor — 出站日志
- **无关**：入站 Processor 链（独立链路，与出站互不干扰）
