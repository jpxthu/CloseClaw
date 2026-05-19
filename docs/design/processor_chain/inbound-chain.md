# 入站链路

## 概述

入站 Processor 链将各 IM 平台的原始消息归一化为统一格式，经过日志记录、会话路由、内容清洗和格式标准化后交付给 Gateway。

入站链路的核心价值在于**平台归一化**：各 IM Adapter 的入站部分将平台特定格式转为 NormalizedMessage，后续所有 Processor 只操作这个统一结构，不感知平台差异。

## 架构

入站链由 IM Adapter 前端和 Processor 链后端组成：

```
IM 平台 webhook 到达
  ↓
IM Adapter（入站）— 平台特定解析
  ├── 飞书 → 解析 webhook JSON，提取消息体和元数据
  ├── Discord → 解析 Gateway event
  └── Telegram → 解析 update 对象
  ↓
NormalizedMessage（统一中间结构）
  {
    sender_id       — 发送者标识
    content_blocks  — 消息内容（Text / Image / File 等块）
    chat_id         — 会话标识
    thread_id       — 话题标识（可选）
    timestamp       — 消息时间
    platform        — 来源平台标识
  }
  ↓
Processor 链（按 priority 升序执行）
  ├── RawLogProcessor（priority 10）
  │     → 将原始消息写入日志文件（Debug 模式）
  │     → 透传原内容，不改动
  │
  ├── SessionRouter（priority 20）
  │     → 根据 sender_id + chat_id 计算 session key
  │     → 查找或创建 session，将 session_id 写入 metadata
  │
  ├── MessageCleaner（priority 30）
  │     → 移除平台元数据字段
  │     → 富文本消息展开为 markdown（如飞书 post 类型）
  │     → 媒体消息提取描述文本
  │
  └── MarkdownNormalizer（priority 40）
        → 统一换行符
        → 移除不可见字符
        → 标准化引用格式
  ↓
ProcessedMessage → Gateway 路由
```

NormalizedMessage 是 Processor 链的入口格式，由 IM Adapter 产出，内容以 ContentBlock[] 承载。Adapter 不参与链执行——它完成平台解析后退出，链完全由 Processor 驱动。

链出口为 ProcessedMessage（cleaned text + metadata），MessageCleaner 将 ContentBlock[] 提取为清洗后的纯文本，后续 Processor 操作文本字符串。

## 数据流

```
IM webhook JSON / event
  → IM Adapter 解析，构造 NormalizedMessage
    → Processor 链入口：ProcessedMessage { content, metadata }
      → RawLogProcessor：记录原始内容到日志 → 透传
        → SessionRouter：计算 session_id → 写入 metadata
          → MessageCleaner：清洗平台字段，提取纯文本 → 更新 content
            → MarkdownNormalizer：标准化 → 更新 content
              → 输出 ProcessedMessage
                → Gateway 路由：斜杠指令 / 普通消息
```

分支条件：
- 私聊消息：SessionRouter 自动创建新 session
- 群聊 @机器人：标记为群聊消息，session 按群聊维度管理
- 富文本消息（飞书 post 类型）：MessageCleaner 展开为 markdown，标签映射到对应 markdown 语法
- 媒体消息（图片/文件）：提取描述文本，原始资源通过独立下载路径获取

## 模块关系

- **上游**：IM Adapter（提供 NormalizedMessage）
- **下游**：Gateway（接收 ProcessedMessage 并路由）
- **链内**：
  - RawLogProcessor — 日志记录，对内容无改动
  - SessionRouter — session 路由，依赖 session 模块
  - MessageCleaner — 内容清洗，依赖各平台消息格式知识
  - MarkdownNormalizer — 格式标准化
- **无关**：出站 Processor 链（独立链路，与入站互不干扰）
