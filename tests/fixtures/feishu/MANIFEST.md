# Feishu API Fixtures Manifest

每个 JSON 文件对应的场景记录。

## 收集进度

| # | 场景 | 文件名 | msg_type | 说明 |
|---|------|--------|----------|------|
| 1 | 纯文本消息 | `im-message-receive_v1-no-event-id-2026-04-26T18-53-09-967Z.json` | text | 私聊 p2p |
| 2 | @机器人 消息 | `im-message-receive_v1-no-event-id-2026-04-26T18-56-56-983Z.json` | text | 群聊 group，含 mentions 数组 |
| 2a | 机器人加群事件 | `im-chat-member-bot-added_v1-no-event-id-2026-04-26T18-54-46-838Z.json` | - | bot 被加入群聊 |
| 3 | 有序列表 | `im-message-receive_v1-no-event-id-2026-04-27T02-58-21-195Z.json` | post | 私聊 p2p，含嵌套有序/无序列表 |
| 4 | 无序列表 | `im-message-receive_v1-no-event-id-2026-04-27T02-58-21-195Z.json` | post | 同上，同一消息同时覆盖场景 3+4 |
| 7 | 加粗/斜体 | `im-message-receive_v1-no-event-id-2026-04-27T03-07-35-497Z.json` | post | 私聊 p2p，含 bold/strikethrough/underline，无 italic（手机不支持） |
| 9 | 引用块 | `im-message-receive_v1-no-event-id-2026-04-27T03-07-35-497Z.json` | post | 同上，含引用块样式 |
| 5 | 代码块 | | | |
| 6 | 行内代码 | | | |
| 8 | 超链接 | | | |
| 10 | 话题消息（thread） | `im-message-receive_v1-no-event-id-2026-04-27T03-15-32-562Z.json` | text | 私聊 p2p，含 thread_id / parent_id / root_id，thread_id: omt_1a8b5a3fbe4ddbee | |
| 11 | 卡片交互回调 | `card-action-trigger-no-event-id-2026-04-27T03-49-54-956Z.json` | card.action.trigger | 用户点击卡片按钮事件，含 operator / action.tag / context.open_message_id | |
| 12 | 消息撤回 | | | |
| 13 | 表情消息 | `im-message-receive_v1-no-event-id-2026-04-27T03-23-50-544Z.json` | text | 私聊 p2p，含 [OK][Yes][敲键盘][完成][赞] 等飞书 emoji 占位符格式 | |
| 14 | 图片消息 | `im-message-receive_v1-no-event-id-2026-04-27T03-32-13-305Z.json` | post | 私聊 p2p，图片以 `<img>` 标签嵌入 post，image_key: img_v3_02115_89c71a47... | |
| 15 | 文件消息 | | | |
| 16 | 发送文本消息响应 | | | |
| 17 | 发送卡片消息响应 | | | |

## 额外收获（飞书特有样式，清单未列）

| 样式 | 说明 |
|------|------|
| strikethrough（删除线） | `style: ["lineThrough"]` |
| underline（下划线） | `style: ["underline"]` |

## 额外事件类型（不在 17 场景内）

| 事件 | 文件名 | 说明 |
|------|--------|------|
| reaction.created | `im-message-reaction-created_v1-*.json`（共 9 个事件） | 9 种不同 emoji 类型：OK, Yes, Typing, DONE, LightThumbsup, MediumLightThumbsup, MediumThumbsup, MediumDarkThumbsup, DarkThumbsup |
| reaction.deleted | （待收集） | 用户删除 emoji 反应 |

## 字段说明

- `message.message_type`: text / post / image / file / audio / card / interactive 等
- `message.content`: 消息内容 JSON 字符串（text 类型为 `{"text":"..."}`）
- `message.chat_type`: p2p（私聊）/ group（群聊）
- `message.mentions`: @提及列表（群聊 @机器人 时有）
