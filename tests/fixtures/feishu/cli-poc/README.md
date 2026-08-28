# 飞书 CLI 事件 Fixture（lark-cli event consume 采集）

来源：飞书官方 CLI（v1.0.91）`event consume` 长连接输出的 NDJSON 事件，经 ID 占位符脱敏。

与 `../im-message-receive_v1-*.json`（webhook SDK 采集）的差异：

- 字段为**扁平结构**（`chat_id`/`message_id`/`sender_type`/`content` 直接在顶层），非 webhook 的 `event.xxx` 嵌套；`reaction.created_v1` 等**非 receive 类事件仍为信封式**（`schema`/`header`/`event.xxx`），两格式并存
- 每行一个事件（NDJSON），`--output-dir` 落盘为单事件单文件

## 文件清单

| 文件 | 场景 | 关键字段证据 |
|------|------|------------|
| `p2p-text.json` | 私聊顶层文本消息（每条开启新 Session 的对象） | 无 thread 字段 |
| `p2p-topic-reply.json` | 私聊内话题回复（归入既有 Session） | `thread_id` + `root_id` + `reply_to`，三者与根消息 ID 的关系：`root_id` == `reply_to` == 根消息 `message_id` |
| `group-mention.json` | 群内 @ 消息（bot 发送方） | `sender_type: "bot"`；`mentions[]` 的 `id` 为**接收方应用语境**的 open_id（发送方语境的 ID 被平台自动翻译），`content` 内 @ 占位符已替换为显示名 |
| `group-post.json` | 群内富文本（post/markdown）消息 | `content` 为**平文本化**结果：标题并入正文、链接转 `[text](url)`、@ 保留 `<at>` 标签；样式（粗体等）不进事件流 |

## 实测确认的字段语义

- `sender_type`：`user` / `bot`，bot 间互发可编程判定发送方是否为 bot
- `thread_id`：`omt_` 前缀，p2p 与群内格式一致；顶层消息无此字段
- `root_id`：话题根消息 ID（话题内各消息相同）；`reply_to`：直接父消息 ID
- 群消息默认「@才收」：未 @ 的群消息不产生 receive 事件（对 text/post/markdown/卡片一致）
