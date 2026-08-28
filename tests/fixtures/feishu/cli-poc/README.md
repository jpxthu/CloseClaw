# 飞书 CLI 事件 Fixture（lark-cli event consume 采集）

来源：飞书官方 CLI（v1.0.91）`event consume` 长连接输出的 NDJSON 事件。2026-08-28 复测（CloseClaw CLI 复测群 + bot-1/bot-2 双机器人 p2p）采集，18/18 项通过；每份文件为单条原始事件，除「脱敏规则」一节外与采集输出逐字节一致。

本目录数据**逐步替代** `../im-message-receive_v1-*.json` 等 webhook SDK 采集的旧样本（旧样本为 `event.xxx` 嵌套结构）；替代完成前两套并存，解析测试以各自声明为准。

## 与旧 fixture（webhook SDK 采集）的结构差异

- receive 类事件为**扁平结构**（`chat_id`/`message_id`/`sender_type`/`content` 直接在顶层），非 webhook 的 `event.xxx` 嵌套
- `reaction.created_v1`、`card.action.trigger` 等**非 receive 类事件仍为信封式**（`schema`/`header`/`event`/顶层平铺混合），两格式并存，解析层需双格式支持

## 文件清单

| 文件 | 场景 | 关键字段证据 |
|------|------|------------|
| `p2p-top-text.json` | 私聊顶层文本消息（每条开启新 Session 的对象） | 无 thread 字段 |
| `p2p-thread-reply.json` | 私聊内话题回复（归入既有 Session） | `thread_id` + `root_id` + `reply_to`；本例 `root_id` == `reply_to` == 根消息 `message_id` |
| `group-mention-all.json` | 群内 @all（bot 发送方） | `sender_type: "bot"`，`content` 以 `@_all` 开头 |
| `group-mention-bot.json` | 群内定向 @bot（bot 发送方） | `mentions[].id` 为接收方应用语境 open_id，`content` 内占位符已替换为显示名 |
| `group-mention-user-b1view.json` | 同一条用户双@消息，**bot-1 接收视角** | 与 `*-b2view.json` 对照用，见下「跨应用 ID 翻译」 |
| `group-mention-user-b2view.json` | 同一条用户双@消息，**bot-2 接收视角** | 同上 |
| `group-thread-reply.json` | 群内话题回复（bot 发送方） | `thread_id`（`omt_` 前缀）+ `root_id` + `reply_to` 三件套 |
| `group-post-flat.json` | 群内富文本（post）消息 | `content` 为**平文本化**结果：标题并入正文、链接转 `[text](url)`、@ 保留 `<at>` 标签；样式（粗体等）不进事件流 |
| `reaction-created-envelope.json` | 表情回应事件（信封式格式代表） | `schema`/`header.event_type` + `event.message_id`；`operator` 侧为 app_id |
| `card-action-trigger.json` | 卡片按钮点击回调 | `action_tag`/`action_value`/`host`；**单消费者限制**（第二个 consumer 被拒） |

## 跨应用 ID 翻译（b1view / b2view 对照）

`group-mention-user-b1view.json` 与 `group-mention-user-b2view.json` 是**同一条群消息**（同 `message_id`）被两个 bot 分别接收的事件：

- `sender_id`、`mentions[].id` 全部不同——事件中的 open_id 按**接收方应用语境**翻译，同一人/同一 bot 在两个应用语境下 ID 必然不同
- 发送方若要定向 @ 某个 bot，必须使用**目标 bot 在发送方应用语境的 open_id**
- 结论：adapter 必须维护身份映射表（跨应用 open_id 隔离是平台行为，不是配置问题）

## 实测确认的字段语义

- `sender_type`：`user` / `bot`，bot 间互发可编程判定发送方是否为 bot
- `thread_id`：`omt_` 前缀，p2p 与群内格式一致；顶层消息无此字段
- `root_id`：话题根消息 ID（话题内各消息相同）；`reply_to`：直接父消息 ID
- 群消息默认「@才收」：未 @ 的群消息不产生 receive 事件（对 text/post/markdown/卡片一致）
- 图片消息不能与 @ 同条发送：纯图片不触发接收事件，「图+通知」需拆两条消息

## 发送侧

发送命令与实测结论见 `send-commands.md`（CLI 命令面按功能域归类，全部实测通过）。

## 脱敏规则

- **保留**：bot 的 app_id/open_id、群 chat_id、消息 message_id/thread_id、事件 id——测试 bot 与群可随时删除
- **替换**：owner 的 open_id 按接收语境替换为 `<open_id_owner_b1ctx>` / `<open_id_owner_b2ctx>`（保留两个不同占位符，正是为了保住「跨应用 ID 不同」的语义）；`tenant_key` 替换为 `<tenant_key>`
