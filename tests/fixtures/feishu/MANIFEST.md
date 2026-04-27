# Feishu API Fixtures Manifest

全部 21 个 fixture JSON 文件逐一枚举（不含 package.json / package-lock.json）。

## Fixture 文件清单

| 文件名 | event_type | msg_type | 场景/说明 |
|--------|-----------|----------|----------|
| `im-message-receive_v1-no-event-id-2026-04-26T18-53-09-967Z.json` | im.message.receive_v1 | text | 私聊 p2p，纯文本消息 |
| `im-message-receive_v1-no-event-id-2026-04-26T18-56-56-983Z.json` | im.message.receive_v1 | text | 群聊 group，@机器人消息，含 mentions 数组 |
| `im-chat-member-bot-added_v1-no-event-id-2026-04-26T18-54-46-838Z.json` | im.chat.member.bot.added_v1 | — | bot 被加入群聊事件 |
| `im-message-receive_v1-no-event-id-2026-04-27T02-58-21-195Z.json` | im.message.receive_v1 | post | 私聊 p2p，含嵌套有序/无序列表 |
| `im-message-receive_v1-no-event-id-2026-04-27T03-00-16-305Z.json` | im.message.receive_v1 | post | 私聊 p2p，标题+有序列表（不使用/使用富文本） |
| `im-message-receive_v1-no-event-id-2026-04-27T03-07-35-497Z.json` | im.message.receive_v1 | post | 私聊 p2p，含加粗/删除线/下划线、引用块样式 |
| `im-message-receive_v1-no-event-id-2026-04-27T03-15-32-562Z.json` | im.message.receive_v1 | text | 私聊 p2p，话题消息，含 thread_id / parent_id / root_id |
| `im-message-receive_v1-no-event-id-2026-04-27T03-16-01-866Z.json` | im.message.receive_v1 | text | 私聊 p2p，普通文本消息（用于验证消息撤回场景） |
| `im-message-receive_v1-no-event-id-2026-04-27T03-23-50-544Z.json` | im.message.receive_v1 | text | 私聊 p2p，含飞书 emoji 占位符格式（[OK][Yes][敲键盘][完成][赞]） |
| `im-message-receive_v1-no-event-id-2026-04-27T03-32-13-305Z.json` | im.message.receive_v1 | post | 私聊 p2p，图片以 `<img>` 标签嵌入 post |
| `im-message-reaction-created_v1-no-event-id-2026-04-27T03-25-04-327Z.json` | im.message.reaction.created_v1 | — | reaction 事件，emoji_type: OK |
| `im-message-reaction-created_v1-no-event-id-2026-04-27T03-26-18-537Z.json` | im.message.reaction.created_v1 | — | reaction 事件，emoji_type: Yes |
| `im-message-reaction-created_v1-no-event-id-2026-04-27T03-26-22-427Z.json` | im.message.reaction.created_v1 | — | reaction 事件，emoji_type: Typing |
| `im-message-reaction-created_v1-no-event-id-2026-04-27T03-26-27-536Z.json` | im.message.reaction.created_v1 | — | reaction 事件，emoji_type: DONE |
| `im-message-reaction-created_v1-no-event-id-2026-04-27T03-26-34-286Z.json` | im.message.reaction.created_v1 | — | reaction 事件，emoji_type: LightThumbsup |
| `im-message-reaction-created_v1-no-event-id-2026-04-27T03-26-36-077Z.json` | im.message.reaction.created_v1 | — | reaction 事件，emoji_type: MediumLightThumbsup |
| `im-message-reaction-created_v1-no-event-id-2026-04-27T03-26-37-946Z.json` | im.message.reaction.created_v1 | — | reaction 事件，emoji_type: MediumThumbsup |
| `im-message-reaction-created_v1-no-event-id-2026-04-27T03-26-50-954Z.json` | im.message.reaction.created_v1 | — | reaction 事件，emoji_type: MediumDarkThumbsup |
| `im-message-reaction-created_v1-no-event-id-2026-04-27T03-26-56-636Z.json` | im.message.reaction.created_v1 | — | reaction 事件，emoji_type: DarkThumbsup |
| `im-message-reaction-created_v1-no-event-id-2026-04-27T03-45-32-164Z.json` | im.message.reaction.created_v1 | — | reaction 事件，emoji_type: THUMBSUP |
| `card-action-trigger-no-event-id-2026-04-27T03-49-54-956Z.json` | card.action.trigger | — | 卡片交互回调，含 operator / action.tag / context.open_message_id |

## 字段说明

- `event_type`: 飞书事件类型，对应 JSON 根字段 `event_type`
- `msg_type`: `message.message_type`（text / post 等），reaction / card 类事件无此字段
- `im-message.receive` 场景的 `chat_type`（p2p / group）见各文件 `message.chat_type`
- reaction 事件的 emoji 类型见 `message.reaction.emoji_type`

## 场景分类汇总

| 场景 | 文件 |
|------|------|
| 文本消息（p2p） | `im-message-receive_...18-53-09...`，`im-message-receive_...03-15-32...`，`im-message-receive_...03-16-01...` |
| @机器人消息（group） | `im-message-receive_...18-56-56...` |
| 有序/无序列表（post） | `im-message-receive_...02-58-21...`，`im-message-receive_...03-00-16...` |
| 富文本样式（加粗/删除线/下划线/引用） | `im-message-receive_...03-07-35...` |
| 话题消息（thread） | `im-message-receive_...03-15-32...` |
| 表情 emoji 占位符 | `im-message-receive_...03-23-50...` |
| 图片消息（img 标签） | `im-message-receive_...03-32-13...` |
| 消息撤回 | `im-message-receive_...03-16-01...`（发消息后待撤回） |
| bot 加群事件 | `im-chat-member-bot-added_...` |
| 卡片交互回调 | `card-action-trigger_...` |
| reaction 事件（10 种 emoji） | `im-message-reaction-created_...`（共 10 个文件） |
