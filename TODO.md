# TODO — CloseClaw

> 本文件的职责：记录 CloseClaw 项目的功能、bug、改进任务。**完成工作后立即更新此处**。

## Next（当前 Sprint）

- [ ] **Permission Engine 用户维度支持** — 权限配置
  - `subject: user_id` 规则类型
  - 用户级别权限（完整权限 vs 咨询权限）
  - 架构要模块化，方便后续细化
  - **必须配套写测试**
  → **待细化**：需要找你确认权限粒度和矩阵

## Later（低优，可直接开动）

- [ ] **测试文件模块化重构** — 分散到 `src/<module>/tests.rs`
- [ ] **`closeclaw stop -f`** — 强制关闭模式
- [ ] **Hot config reload** — `agents.json` 变更热重载

## 待细化（需先和你对清楚）

- [ ] **Permission Engine 用户维度支持** — 见 Next 第一项
- [ ] **多 IM 适配器优先级** — 企业微信/QQ/钉钉，先做哪个？
- [ ] **Skill 系统设计** — skill review 机制要不要开？

## 待讨论（需先和你讨论）

- [ ] **Feishu webhook server** — 让 daemon 能接收飞书消息（"敲键盘"状态 emoji 依赖此功能）
- [ ] **Graceful shutdown drain 逻辑** — 等 agent 任务完成再退出
- [ ] **Agent 间通信** — 群聊中互相 @ 对话
- [ ] **私聊/群聊 @ 机器人触发对话** — thread 模型设计
- [ ] **`/new` 开新会话** — thread 隔离上下文

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
