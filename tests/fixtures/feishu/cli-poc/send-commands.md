# 发送侧实测操作清单（CLI 命令面）

以下命令在 PoC 中全部实测成功（`ok:true`），按功能域归类：

## 发送

- p2p 文本：`lark-cli im +messages-send --user-id ou_xxx --text "..." --as bot`（open_id 为接收方在该发送方应用语境的 ID）
- 群文本+@：`lark-cli im +messages-send --chat-id oc_xxx --text '<at user_id="all"></at> ...' --as bot`（@all 与定向 @user_id 均实测）
- 群 post：`lark-cli im +messages-send --chat-id oc_xxx --msg-type post --content '{"zh_cn":{"title":"...","content":[[{"tag":"text","text":"...","style":["bold"]},...]]}}'`（style 枚举：bold/italic/lineThrough/underline；非法 style 报 230001 scheme not in whitelist）
- 群 markdown：`lark-cli im +messages-send --chat-id oc_xxx --markdown '...'`（落地为 post 类型）
- 交互卡片：`lark-cli im +messages-send --chat-id oc_xxx --msg-type interactive --content '{"schema":"2.0","body":{"elements":[...]}}'`（最小 element：markdown；button 带 behaviors callback；select_static 带 options）
- 图片/文件：`lark-cli im +messages-send --chat-id oc_xxx --image ./rel.png`、`--file ./rel.txt`（自动上传；仅收 cwd 相对路径）
- 话题回复：`lark-cli im +messages-reply --message-id om_xxx --text "..." --reply-in-thread`
- 表情回应：`lark-cli im reactions create --params '{"message_id":"om_xxx"}' --data '{"reaction_type":{"emoji_type":"OK"}}'`

## 更新（流式）

- 卡片内容更新：`lark-cli im messages patch --message-id om_xxx --data @patch.json`（patch.json = `{"content":"<卡片JSON字符串>"}`；内联转义易损坏，必须 @文件）
- cardkit 流式打字机：
  1. `lark-cli api POST /open-apis/cardkit/v1/cards --data '{type:card_json, data:"<卡片JSON, streaming_mode:true, element_id 必填>"}'`
  2. 发消息引用卡片：`+messages-send --msg-type interactive --content '{"type":"card_json","card_id":"..."}'`
  3. `lark-cli api PUT /open-apis/cardkit/v1/cards/<card_id>/elements/<element_id>/content --data '{"content":"...","uuid":"...","sequence":N}'`（sequence 必填；1000 次/分限流实测余量充足）

## 群管理

- 建群：`lark-cli im +chat-create --name X --as bot --bots cli_xxx --users ou_xxx --set-bot-manager`（`--chat-mode topic` 建话题群）
- 解散群：`lark-cli api DELETE /open-apis/im/v1/chats/<chat_id>`（需 im:chat:delete）
- 移除成员：`lark-cli im chat.members delete --chat-id oc_xxx --data '{"id_list":["cli_xxx"]}' --params '{"member_id_type":"app_id"}' --yes`（high-risk-write，群主不可被移出 232076）

## 事件消费

- `lark-cli event consume im.message.receive_v1 --as bot --profile P --max-events N --timeout 90s [--output-dir .]`（NDJSON 到 stdout；--output-dir 仅相对路径，落盘 `<ns>_<pid>_<seq>.json` 0600）
- `lark-cli event consume card.action.trigger ...`（单消费者限制）
- stderr 就绪标记 `[event] ready`；bus daemon 自动启停（30s 无消费者自退）

## 权限/身份

- `lark-cli profile add --name X --app-id cli_xxx --app-secret-stdin`；`profile list/use/remove`
- `lark-cli auth status`（bot/user 身份分列）
- scope 申请：报错附 console_url（预填 missing_scopes），owner 后台批准
