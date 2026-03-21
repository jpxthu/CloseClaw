# TODO — CloseClaw

## Next（当前 Sprint）

- [ ] **Chat 命令** — 本地 CLI 直连 daemon，不依赖 IM
  - TCP localhost 监听（127.0.0.1:18889）
  - JSON over TCP 协议
  - `closeclaw chat` CLI 客户端
  - 默认启动 guide agent
  - 验证核心架构可行

- [ ] **Permission Engine 用户维度支持** — 权限配置
  - `subject: user_id` 规则类型
  - 用户级别权限（完整权限 vs 咨询权限）
  - 架构要模块化，方便后续细化

## Later（待讨论/低优）

- [ ] **Feishu webhook server** — 让 daemon 能接收飞书消息
- [ ] **Graceful shutdown drain 逻辑** — 等 agent 任务完成再退出
- [ ] **`closeclaw stop -f`** — 强制关闭模式
- [ ] **Agent 间通信** — 群聊中互相 @ 对话（低优，需进一步讨论）
- [ ] **Hot config reload** — `agents.json` 变更热重载
- [ ] **接入 LLM provider** — OpenAI / Anthropic / MiniMax 实际调用

## 功能体验（来自用户反馈）

### 必须做（核心体验）
| 功能 | 说明 |
|------|------|
| 私聊/群聊 @ 机器人触发对话 | CloseClaw 必须实现 |
| "敲键盘" 状态 emoji | Feishu webhook 需要 |
| `/new` 开新会话 | thread 隔离上下文 |

### 避免踩坑（来自 OpenClaw 体验差的问题）
| 问题 | CloseClaw 对策 |
|------|--------------|
| 代码块显示为普通文字 | 确认 markdown 渲染正确 |
| 私聊和话题混在一起 | 设计好 thread 模型 |
| 流式输出只看到最后一条 | streaming 逐条渲染 |
| 指令执行中 restart 丢消息 | graceful shutdown |
| 主动消息收不到 | 需要测试验证 |

### 权限配置（用户维度）
```
员工 A → 完整权限（文件变更、配置变更、agent 管理）
员工 B/C/D → 咨询权限（只读，不能变更）
```
> 硬性规则形式，不是 prompt。模块化架构预留。

## 已完成 ✅

| 功能 | Commit |
|------|--------|
| Permission Engine | b1279e6 |
| Agent Registry + Process | a5a247b |
| Gateway + Feishu Adapter | 24c6f4d |
| CLI: stop 命令 | 51d7976 |
| Daemon 启动框架 | 4836d9e |
| Graceful Shutdown 状态机 | 4836d9e |
| `closeclaw config setup` | 170942e |
| Issue #1 文档中文化 | 06f5f63 |
| Issue #2 API Key 向导 | 170942e |
| Issue #3 config setup 修复 | 1c86cc5 |

---

*由 Vibe虾 🦐 维护 | 最后更新: 2026-03-22*
