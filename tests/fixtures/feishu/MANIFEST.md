# Feishu API Fixtures Manifest

每个 JSON 文件对应的场景记录。

## 收集进度

| # | 场景 | 文件名 | msg_type | 说明 |
|---|------|--------|----------|------|
| 1 | 纯文本消息 | `im-message-receive_v1-no-event-id-2026-04-26T18-53-09-967Z.json` | text | 私聊 p2p |
| 2 | @机器人 消息 | `im-message-receive_v1-no-event-id-2026-04-26T18-56-56-983Z.json` | text | 群聊 group，含 mentions 数组 |
| 2a | 机器人加群事件 | `im-chat-member-bot-added_v1-no-event-id-2026-04-26T18-54-46-838Z.json` | - | bot 被加入群聊 |
| 3 | 有序列表 | | | |
| 4 | 无序列表 | | | |
| 5 | 代码块 | | | |
| 6 | 行内代码 | | | |
| 7 | 加粗/斜体 | | | |
| 8 | 超链接 | | | |
| 9 | 引用块 | | | |
| 10 | 话题消息（thread） | | | |
| 11 | 卡片交互回调 | | | |
| 12 | 消息撤回 | | | |
| 13 | 表情消息 | | | |
| 14 | 图片消息 | | | |
| 15 | 文件消息 | | | |
| 16 | 发送文本消息响应 | | | |
| 17 | 发送卡片消息响应 | | | |

## 字段说明

- `message.message_type`: text / post / image / file / audio / card / interactive 等
- `message.content`: 消息内容 JSON 字符串（text 类型为 `{"text":"..."}`）
- `message.chat_type`: p2p（私聊）/ group（群聊）
- `message.mentions`: @提及列表（群聊 @机器人 时有）
