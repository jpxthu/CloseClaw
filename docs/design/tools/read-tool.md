# Read 工具

## 概述

Read 工具为 agent 提供读取文件内容的能力，支持文本文件和图片文件。核心设计目标：引导 agent 分段读取大文件，避免单次读取耗尽上下文窗口。通过截断策略、续读指引和去重缓存，让 agent 高效、准确地消费文件内容。

## 架构

Read 工具的执行链路包含四层处理：

```
输入参数解析（path + 可选 offset/limit）
  → 路径校验（文件存在性、可读性、类型识别）
    → 文本文件：offset/limit 定位 → 累积读取 → 截断检查
    → 图片文件：格式识别 → resize → 作为附件返回
      → 结果组装（内容 + 截断提示 + 续读指引）
```

### 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 文件路径，绝对路径或相对于工作目录 |
| `offset` | number | 否 | 起始行号（1-indexed），不传则从第 1 行开始 |
| `limit` | number | 否 | 最大读取行数，不传则由截断阈值控制 |

offset 和 limit 默认都不传——小文件一次读完，大文件由截断阈值自然触发分段读取。

### 文本文件读取

文本文件读取采用**保留头部、丢弃尾部**的策略。从 offset 指定的行开始（默认第 1 行），逐行累积输出，直至触发任一截断阈值后停止。

### 截断阈值

三重阈值，任一触发即停止读取：

| 阈值 | 默认值 | 说明 |
|------|--------|------|
| 行数上限 | 2000 行 | 单次读取最多返回的行数 |
| 字节上限 | 50KB | 单次读取最多返回的字节数 |
| Token 上限 | 可配置 | 防止单个文件消耗过多上下文 |

阈值优先触发顺序：Token > 字节 > 行数。即先检查 Token 是否超限，再检查字节，最后检查行数。

### 截断后提示

截断不是失败——它必须附带明确的续读指引，告诉 agent 下一步怎么做：

- **按行数截断**：`[Showing lines {start}-{end} of {total}. Use offset={N} to continue.]`
- **按字节截断**：`[Showing lines {start}-{end} of {total} ({size} limit). Use offset={N} to continue.]`
- **用户传了 limit 未读完**：`[{remaining} more lines in file. Use offset={N} to continue.]`
- **首行即超字节上限**：`[Line {N} is {size}, exceeds {limit} limit. Use bash: sed -n '...']`

截断提示给出具体的 offset 数值，agent 不需要自己计算下一步应该从哪里读。

### 文件去重

读取文件后记录该文件的 mtime 和已读的 range。当 agent 在同一 turn 内再次读取同一文件的同一 range 且文件未变更时，返回文件未变更的简短提示而非重新读取内容。mtime 变更视为缓存失效，正常读取。

去重仅针对同一 turn 内的重复调用，不跨 turn 保持——agent 下一轮对话重新获取上下文时，去重状态重置。

### 图片读取

Read 工具也支持图片文件（jpg、png、gif、webp）。图片作为附件发送给 LLM，自动 resize 到合理尺寸（如 2000x2000 上限），避免超出模型输入限制。

图片不适用 offset/limit——始终全量发送。

### 运行时标记

Read 工具标记为只读工具和并发安全工具。多个 Read 调用可以并行执行，互不干扰。

## 数据流

```
agent 调用 ReadTool（path + 可选 offset/limit）
  → 路径校验：文件存在？可读？
    → 不存在 / 不可读 → 返回错误
  → 文件类型识别
    ├─ 文本文件：
    │   → 从 offset 行开始（默认第 1 行）
    │   → 逐行读取，累积字符计数
    │   → 每行检查三重阈值（Token → 字节 → 行数）
    │   → 触发阈值 → 停止读取，附加截断提示和下一个 offset
    │   → 未触发阈值 → 读完整个文件，返回完整内容
    │   → 文件内容作为字符串返回
    └─ 图片文件：
        → resize（不超 2000x2000）
        → 作为附件发送给 LLM
        → 不适用 offset/limit
  → 记录去重状态（path + mtime + range）
  → 返回执行结果给 agent
```

agent 收到截断提示后的典型行为：

```
agent 读到 "[Showing lines 1-2000 of 5230. Use offset=2001 to continue.]"
  → agent 调用 ReadTool(path, offset=2001) 继续读取
    → 重复上述流程
      → 直到全部读完或 agent 获得足够信息
```

## 模块关系

- **上游**：agent 运行时（调度工具调用、传递参数）
- **下游**：文件系统（直接读取文件）、会话上下文（读取结果注入 agent 对话流）
- **无关**：权限引擎（读取为只读操作，不涉及权限审批）、processor_chain（不参与消息出站处理）、IM 适配器（不参与平台渲染）

### 与模块内其他子功能的关系

- **Write/Edit 工具**：Read 先于 Edit——agent 必须读过文件才能精确编辑。Edit 工具执行前的 staleness 检查依赖 Read 工具记录的 mtime
- **Bash 工具**：Bash 输出超过累积阈值时持久化到磁盘，agent 通过 Read 工具读取完整输出
