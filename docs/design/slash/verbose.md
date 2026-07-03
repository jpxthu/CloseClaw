# 信息展示等级

## 概述

`/verbose` 指令用于查询或设置当前会话的信息展示等级。展示等级控制出站链路向用户展示的 Agent 内部工作细节量，不影响 LLM 推理深度，不影响 Agent 行为模式。

## 架构

Verbosity 是出站 Processor Chain 的第一个 Processor（VerbosityFilter，priority 5），在回复内容进入 DSL 解析之前执行过滤。Agent 照常工作——LLM 按设定的推理深度执行推理、按需调用工具、Plan 工作流按规则运转——但 VerbosityFilter 根据展示等级决定哪些内容块进入后续出站链路。

三个等级：

- `full`（默认）：展示全部——思考过程、工具调用、工具结果、最终回复
- `normal`：展示工具调用和结果作为进度提示，隐藏思考过程
- `off`：仅展示最终回复，隐藏所有中间过程

展示等级由 Session 存储，出站过滤层在回复内容进入 Processor Chain 前读取并过滤。切换等级不影响当前正在输出的消息——只对后续新消息生效。

```
/verbose normal
  ↓
处理器返回设置展示等级结果（目标等级 Normal）
  ↓
Gateway 将等级写入 Session
  ↓
回复用户已设置的展示等级
  ↓
下一条回复 → 出站过滤层按等级过滤：
  ├── 思考过程 → 不展示
  ├── 工具调用 → 展示
  ├── 工具结果 → 展示
  └── 最终回复 → 展示
```

### 与 Thinking、Streaming、Plan 模式的关系

三者与 Verbosity 各自独立，可任意组合：

| 轴 | 指令 | 控制什么 | 与 Verbosity 关系 |
|---|---|---|---|
| Thinking | `/reasoning` | LLM 推理深度（API 参数） | 互不影响。推理深度设为高等级 + 展示等级设为 off = 深度推理但只看最终结果 |
| Streaming | 无独立指令 | 传输方式（流式/非流式） | 互不影响。Verbosity 过滤在流式和非流式路径均生效 |
| Plan 模式 | `/plan` `/mode` | Agent 行为模式（工作流 + 工具集） | 互不影响。切换到 Plan 模式 + 展示等级设为 off = 按规划工作流执行但只看最终规划结果 |

## 数据流

- **查询**（无参数）：读取 Session 当前展示等级 → 回复当前等级
- **设置**（带等级参数）：解析等级 → 将等级写入 Session 存储

出站过滤链路（VerbosityFilter 是出站 Processor Chain 的第一个 Processor，priority 5）：

```
Agent 回复内容（含思考块、工具调用块、文本块）
  ↓
读取 Session 存储的展示等级，按等级过滤：
  ├── full → 不过滤，全部内容块通过
  ├── normal → 移除 Thinking 内容块，保留工具调用和文本
  └── off → 仅保留 Text 块，移除 Thinking/ToolUse/ToolResult
  ↓
过滤后内容继续进入 Processor Chain 后续处理
  ↓
Processor Chain 处理（DslParser → OutboundRawLog）
  ↓
Gateway 记录出站日志
  ↓
经处理后消息 → IM 插件渲染 → 发送
```

## 模块关系

- **上游**：Gateway 通过 Dispatcher 路由到 VerbosHandler 处理指令
- **下游**：Session 模块（存储和读取展示等级）；出站 Processor Chain（VerbosityFilter 作为链的第一个 Processor 执行过滤，过滤后内容进入 DslParser → OutboundRawLog）
- **无关**：LLM 模块（Verbosity 是纯客户端过滤，不改变 API 参数）、Plan 模式切换（独立的行为控制）、Streaming 路径（过滤逻辑在 Processor Chain 入口统一执行，与流式/非流式无关）
