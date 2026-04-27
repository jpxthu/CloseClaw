# Feishu API Fixtures

真实飞书消息 API 响应样本，用于 closeclaw 飞书模块的测试开发。

## 获取方式

参考 issue #381：运行捕获脚本 → 脚本自动连接飞书 WebSocket → 发消息 → 脚本自动落盘。

## 准备工作

```bash
# 安装依赖（已在仓库内）
cd tests/fixtures/feishu
npm install
```

## 脚本使用

```bash
# 运行捕获脚本（WebSocket 模式，不需要 ngrok，不需要公网 URL）
node capture_feishu.js <app_id> <app_secret>

# 示例
node capture_feishu.js cli_a964e565a9f8dcb3 PD2GqeCIgxWS1jVhbC1pHbgPwRNQFkVU
```

### 工作原理

使用 Lark SDK 的 `WSClient`（WebSocket 模式）：
- SDK 主动连接 `wss://` 端点，**不需要 ngrok 或公网 URL**
- 不需要验证 Token
- 连接由客户端发起，安全可靠

### 触发收集

运行脚本后，在飞书中给机器人发以下消息：

| # | 场景 | 操作 |
|---|------|------|
| 1 | 纯文本消息 | 发一条普通文字消息给 bot |
| 2 | @机器人 消息 | 在群聊中 @bot |
| 3 | 有序列表 | 发一个数字列表格式的消息 |
| 4 | 无序列表 | 发一个 bullet list |
| 5 | 代码块 | 发一个 \`\`\` 包裹的代码块 |
| 6 | 行内代码 | 发带反引号的文字 |
| 7 | 加粗/斜体 | 发带 \*\*bold\*\* 或 \*italic\* 的文字 |
| 8 | 超链接 | 发一个带链接的消息 |
| 9 | 引用块 | 在群聊/私聊里引用回复一条消息 |
| 10 | 话题消息（thread） | 在一个 thread 里回复 |
| 11 | 卡片按钮点击 | 点击 bot 发出的卡片按钮 |
| 12 | 消息撤回 | 撤回你发的一条消息 |
| 13 | 表情消息 | 发一个 emoji |
| 14 | 图片消息 | 发一张图片给 bot |
| 15 | 文件消息 | 发一个文件给 bot |
| 16 | 发送文本消息响应 | 观察 bot 回复，EDA capture 发送侧响应 |
| 17 | 发送卡片消息响应 | 观察 bot 发出卡片，EDA capture 发送侧响应 |

## 文件命名

`{event_type}-{event_id}-{timestamp}.json`

## API 文档索引

### 消息接收（webhook）

- **事件订阅总览**: https://open.feishu.cn/document/server-docs/im-v1/message/events/receive
- **接收消息**: https://open.feishu.cn/document/server-docs/im-v1/message/receive

### 消息发送

- **发送消息**: https://open.feishu.cn/document/server-docs/im-v1/message/create
- **发送文本消息**: https://open.feishu.cn/document/server-docs/im-v1/message/create#expire-100
- **发送卡片消息**: https://open.feishu.cn/document/server-docs/im-v1/message/create#expire-200
- **更新卡片消息**: https://open.feishu.cn/document/server-docs/im-v1/message/update

### 消息管理

- **撤回消息**: https://open.feishu.cn/document/server-docs/im-v1/message/delete
- **获取消息**: https://open.feishu.cn/document/server-docs/im-v1/message/get

### 认证

- **获取 tenant_access_token**: https://open.feishu.cn/document/server-docs/authen-management/access-token/tenant_access_token_internal

### 完整 API 列表

- **服务端 API 总览**: https://open.feishu.cn/document/server-docs/api-call-guide/server-api-list

## 关键发现记录

（待收集后填写）
