# 飞书渲染器

## 概述

FeishuRenderer 是 Renderer 抽象的飞书平台实现。它读取 Session 中的统一消息格式，按飞书消息规范渲染为 text 或 interactive 卡片格式。

## 架构

FeishuRenderer 的渲染分两步完成：

```
Session 消息数组 + DSL 解析结果
  ↓
步骤一：判断输出类型
  → 纯文本、无格式 → text 消息
  → 含格式标记、多内容块、或 DSL 指令 → interactive 卡片
  ↓
步骤二：组装卡片
  → 提取标题、正文、按钮等元素
  → 按飞书卡片规范生成 JSON
  ↓
RenderedOutput { msg_type, payload }
```

**输出类型判断规则**：

| 条件 | 输出 |
|------|------|
| 纯文本，无换行、无格式标记、无 DSL | text 消息 |
| 含标题标记 | interactive 卡片 |
| 含粗体 / 斜体 / 代码块 / 列表 / 引用 / 链接 | interactive 卡片 |
| 含换行符 | interactive 卡片 |
| 含 DSL 按钮指令 | interactive 卡片 |

**ContentBlock 渲染映射**：

| 内容类型 | 飞书渲染方式 |
|---------|------------|
| 文本（含标题标记） | 提取为卡片 header.title |
| 文本（普通文本与格式） | 卡片正文内容 |
| 文本（分隔线） | 水平分割线元素 |
| 推理内容 | 加粗斜体显示，标注为推理内容 |
| 工具调用 | 工具调用描述信息 |

## 数据流

```
Renderer.render(messages, dsl_result)
  ↓
遍历消息数组，处理每条消息的内容块
  → 对每个块按类型选择渲染策略
  → 同时处理 DSL 指令（按钮等交互元素）
  ↓
组装飞书卡片 JSON：
  → header：标题信息
  → body：文本内容、格式标记
  → actions：DSL 指令对应的交互按钮
  ↓
RenderedOutput { msg_type: "text" | "interactive", payload: card_json }
```

## 模块关系

- **上游**：Gateway（提供 Session 消息数据和 Processor 链的 DSL 解析结果）
- **下游**：飞书 IM Adapter（接收渲染后的文本或卡片 JSON 并发送）
- **同层**：其他平台 Renderer 实现，共享同一个 Renderer 抽象
