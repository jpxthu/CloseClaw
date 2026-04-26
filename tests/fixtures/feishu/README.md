# Feishu API Fixtures

真实飞书消息 API 响应样本，用于 closeclaw 飞书模块的测试开发。

## 获取方式

参考 issue #381：写捕获脚本 → 你运行脚本 → 你发消息 → 脚本自动落盘。

## 脚本使用

```bash
# 1. 运行脚本（需要 python3）
python3 capture_feishu.py <端口> <verification_token>

# 示例
python3 capture_feishu.py 8080 your-verification-token

# 2. 用 ngrok 暴露到公网
ngrok http 8080

# 3. 复制 ngrok 的 Forwarding URL（https://xxx.ngrok.io）
#    粘贴到飞书开放平台 → 事件订阅 → 请求地址 URL

# 4. 给机器人发各种消息，脚本自动保存 JSON 到本目录
```

## 文件命名

`{event_type}-{event_id}-{timestamp}.json`

## 待收集场景清单

| # | 场景 | 说明 |
|---|------|------|
| 1 | 纯文本消息 | 普通文字私聊 |
| 2 | @机器人 消息 | 在群聊中 @你的 bot |
| 3 | 有序列表 | 数字列表格式 |
| 4 | 无序列表 | bullet list |
| 5 | 代码块 | \`\`\` 包裹的代码 |
| 6 | 行内代码 | 反引号高亮 |
| 7 | 加粗/斜体 | \*\*bold\*\* / \*italic\* |
| 8 | 超链接 | 带链接的文字 |
| 9 | 引用块 | 回复引用消息 |
| 10 | 话题消息 | 在 thread 内回复 |
| 11 | 卡片按钮点击 | 点击 bot 发出的卡片 |
| 12 | 消息撤回 | 撤回一条消息 |
| 13 | 表情消息 | emoji |
| 14 | 图片消息 | 发一张图片 |
| 15 | 文件消息 | 发一个文件 |

## 关键发现记录

（待收集后填写）
