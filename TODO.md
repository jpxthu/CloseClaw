# TODO — CloseClaw

## Next

- [ ] **Feishu webhook server** — 让 daemon 能真正接收飞书消息（webhook endpoint + 签名验证）
- [ ] **Graceful shutdown drain 逻辑** — 等 agent 任务完成再退出，接入 `ShutdownHandle`
- [ ] **`closeclaw stop -f`** — 强制关闭模式，SIGKILL 立即终止

## In Progress

- [ ] **Daemon 核心完善** — 目前 daemon 已启动但还没有实际的消息处理循环

## Later

- [ ] **Agent 间通信** — Registry 里的 agent 互相发消息
- [ ] **Hot config reload** — `agents.json` 变更自动热重载（不用 restart）
- [ ] **接入 LLM provider** — OpenAI / Anthropic / MiniMax 实际调用
- [ ] **Permission 规则加载** — 从 `permissions.json` 加载 RuleSet 到 PermissionEngine
- [ ] **子 agent 进程管理** — `AgentRegistry::spawn` 实际 fork 子进程

---

## 已完成 ✅

| 功能 | 状态 |
|------|------|
| Permission Engine | ✅ |
| Agent Registry + Process | ✅ |
| Gateway + Feishu Adapter | ✅ |
| CLI: stop 命令 | ✅ |
| Daemon 启动框架 | ✅ |
| Graceful Shutdown 状态机 | ✅ |
| Feishu adapter 注册（env 方式） | ✅ |

---

*由 Vibe虾 🦐 维护 | 最后更新: 2026-03-22*
