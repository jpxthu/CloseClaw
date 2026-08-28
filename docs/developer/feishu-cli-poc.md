# 飞书 CLI PoC 测试记录

> 2026-08-28 对飞书官方 CLI（`lark-cli` v1.0.91，[github.com/larksuite/cli](https://github.com/larksuite/cli)）的实测记录，为 [im_adapter/feishu](../requirements/im_adapter/feishu.md) 需求提供事实依据。本文所有结论均来自双机器人真实环境实测；测试账号凭证不落库。

## 测试环境与方法

- 两个飞书自建应用（测试机器人）+ 单机 CLI，双 bot 互为收发对端
- 接收：`lark-cli event consume`（本地 WebSocket 长连接，bus daemon 自动启停）
- 发送：`lark-cli im +messages-send / +messages-reply`、cardkit API（流式卡片）
- 测试场景覆盖：p2p、群聊（@触发）、话题（p2p 与话题群）、媒体、表情回应、卡片（展示/流式/交互）、错误路径

## 核心结论（支撑需求变更）

| # | 结论 | 对应需求 |
|---|---|---|
| 1 | 长连接事件订阅可稳定收消息，无需公网回调（70 分钟连续收发零失败；12h 级稳定性留集成阶段观察） | feishu F1 |
| 2 | 事件字段 `thread_id`/`root_id`/`reply_to` 齐全；p2p 与群内话题行为一致；私聊内可多话题并行 | feishu F7（Session 按话题划分） |
| 3 | 发送方 @ 的用户 ID 在接收方事件中自动翻译为**接收方应用语境**的 ID；`mentions[]` 携带显示名 | 归一化身份字段设计输入 |
| 4 | 事件 content 对 post/markdown 消息输出**平文本化**结果（样式不进事件流） | feishu F1 富文本降级 |
| 5 | 卡片可 p2p 直发；流式更新走 cardkit 三步（建卡片实体 → 消息引用 card_id → PUT 推文本），打字机效果实测可用 | feishu F4（流式卡片基础） |
| 6 | bot 无法直接给消息加"已读"（read 类命令均为查询）；表情回应事件可作轻量确认信道 | feishu F2 事件范围 |
| 7 | 错误路径 7 类实测，错误信封统一（`ok:false` + type/subtype/code + log_id），本地校验类错误不发网络请求 | feishu 可靠性 |

## 限制与边界（实测确认的平台约束）

- **open_id 按「应用×会话」隔离**：同一用户在不同应用、甚至同应用不同群的 ID 均不同；跨应用直接以 open_id 寻址会报 `99992361 open_id cross app`
- **bot 之间不能 p2p**：bot 无联系人概念，bot↔bot 通信只能走共享群 + @（或话题）
- **群消息默认「@才收」**：未 @ 的群消息（含 thread 回复、纯卡片）不触发事件；与"群聊暂不设计"的现状无冲突
- **卡片按钮回调**（`card.action.trigger`）需平台侧额外配置交互回调订阅，本轮未跑通，与需求 F2"暂不开发"备注一致
- **card.action.trigger 单消费者**：同一订阅只允许一个消费进程

## 错误码速查（实测）

| 场景 | code | 语义 |
|---|---|---|
| chat_id 不存在 | 230001 | invalid receive_id |
| open_id 跨应用 | 99992361 | open_id cross app |
| scope 未申请 | 99991672 | 附 missing_scopes 清单 |
| bot 无资源权限 | 230027 | access denied for this bot operation |
| 群主不可被移出 | 232076 | Can't kick chat owner |
| 本地参数校验失败 | （无 code） | validation，不发网络请求 |

长文本无 1500 字限制（2 万字实测发送成功）。

## 遗留观察项（不阻塞需求，集成阶段关注）

- 长连接断线重连与 token 过期（2h）行为：短周期测试未复现异常，长跑由 adapter 集成后随实际运行观察
- 群内全量消息接收权限、话题群（chat_mode=topic）行为：未测（群聊暂不在需求范围）
