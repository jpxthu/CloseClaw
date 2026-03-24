# TODO — CloseClaw

> 本文件的职责：记录 CloseClaw 项目的功能、bug、改进任务。**完成工作后立即更新此处**。

## Next（当前 Sprint）

- [x] **[Bug] user_scope_test.rs 编译错误** → [#49](https://github.com/jpxthu/CloseClaw/issues/49) ✅ 已由 Builder 确认修复
- [ ] **GitHub Actions CI 流程** → [#42](https://github.com/jpxthu/CloseClaw/issues/42) (role:process)
- [ ] **集成测试用例补充** → [#40](https://github.com/jpxthu/CloseClaw/issues/40) (role:builder)
- [ ] **代码审计：closeclaw stop -f** → [#38](https://github.com/jpxthu/CloseClaw/issues/38) (role:builder)
- [ ] **代码审计：Permission Engine 用户维度** → [#36](https://github.com/jpxthu/CloseClaw/issues/36) (role:reviewer)
- [ ] **代码审计：closeclaw chat TCP 框架** → [#37](https://github.com/jpxthu/CloseClaw/issues/37) (role:reviewer)

## Later（低优，可直接开动）

- [x] **`closeclaw chat`** — 本地 CLI 直连 daemon，TCP + LLM provider ✅ (commit 2470765)

- [ ] **测试文件模块化重构** — 分散到 `src/<module>/tests.rs` → [#16](https://github.com/jpxthu/CloseClaw/issues/16) ⚠️ 无实现 commit
- [ ] **测试流程规范化** — UT + 集成测试 + 测试员手动验收 + 自动化测试沉淀 → [#21](https://github.com/jpxthu/CloseClaw/issues/21) ⚠️ 无实现 commit
- [x] **`closeclaw stop -f`** — 强制关闭模式 ✅ (commit 5e82c75)
- [ ] **Hot config reload** — `agents.json` 变更热重载 → [#17](https://github.com/jpxthu/CloseClaw/issues/17)
- [ ] **Streaming 逐条渲染** — CLI 输出时 streaming 响应逐条显示 → [#18](https://github.com/jpxthu/CloseClaw/issues/18)
- [ ] **代码块 markdown 渲染** — CLI 和 IM 输出中代码块正确渲染 → [#19](https://github.com/jpxthu/CloseClaw/issues/19)
- [ ] **Phase 11 日志与审计系统** — 权限判断和 agent 操作全记录 → [#20](https://github.com/jpxthu/CloseClaw/issues/20)
- [ ] **OpenClaw 配置热重载** — 改 openclaw.json 不应断开当前 session → [#23](https://github.com/jpxthu/CloseClaw/issues/23)

## 待细化（需先和你对清楚）

- [ ] **Permission Engine 用户维度支持** — 见 Next 第一项
- [ ] **多 IM 适配器优先级** — 企业微信/QQ/钉钉，先做哪个？
- [ ] **Skill 系统设计** — skill review 机制要不要开？

## 待讨论（需先和你讨论）

- [ ] **Feishu webhook server** — 让 daemon 能接收飞书消息 → [#24](https://github.com/jpxthu/CloseClaw/issues/24)
- [ ] **Graceful shutdown drain 逻辑** — 等 agent 任务完成再退出 → [#25](https://github.com/jpxthu/CloseClaw/issues/25)
- [ ] **Agent 间通信** — 群聊中互相 @ 对话 → [#26](https://github.com/jpxthu/CloseClaw/issues/26)
- [ ] **私聊/群聊 @ 机器人触发对话** — thread 模型设计 → [#27](https://github.com/jpxthu/CloseClaw/issues/27)
- [ ] **`/new` 开新会话** — thread 隔离上下文 → [#28](https://github.com/jpxthu/CloseClaw/issues/28)
- [ ] **配置版本管理** — workspace 配置导出、Git 跟踪、跨机器同步 → [#28](https://github.com/jpxthu/CloseClaw/issues/28)

## 多层级 Agent 架构设计（已完成）✅

| 文档 | 说明 |
|------|------|
| `docs/agent/MULTI_AGENT_ARCHITECTURE.md` | 层级架构、权限系统、通讯机制、经验共享完整设计 |
| `docs/agent/README.md` | Agent 模块文档索引（已更新） |

**已确认设计：**
- Agent 权限配置在各自目录下，Agent 可读不可改
- 通讯名单由 CloseClaw 中央仲裁
- 经验类型由父 Agent 最终判定
- max_depth 由 CloseClaw 逻辑校验

**TODO（后续再定）：**
- 经验推送机制（父→子的下行推送实现）
- 通讯延迟处理（消息队列/长连接/拉取策略）

## 待实现（设计明确，可自行开始）

- (已全部完成 ✅ — 见下方 Commit)

## 已完成（来自待实现列表）✅

| 功能 | Commit |
|------|--------|
| Agent 配置文件结构定义 | d8a4d3f |
| Agent 配置文件加载/保存 | 5e33754 |
| PermissionEngine.check(agent_id, action) API | ddaa453 |
| Communication List 中央仲裁逻辑 | 14cee8c |
| max_depth 层级校验逻辑 | 0b88aac |
| AgentRegistry parent_id 层级支持 | 5fa8a87 |
| permission_query 内置 SKILL | 91a2edd |

## 已完成 ✅

| 功能 | Commit |
|------|--------|
| Permission Engine | b1279e6 |
| Permission Engine 用户维度支持（Subject::UserAndAgent, Creator Rule, Template 系统） | — |
| Agent Registry + Process | a5a247b |
| Gateway + Feishu Adapter | 24c6f4d |
| CLI: stop 命令 | 51d7976 |
| Daemon 启动框架 | 4836d9e |
| Graceful Shutdown 状态机 | 4836d9e |
| `closeclaw config setup` | 170942e |
| Issue #1 文档中文化 | 06f5f63 |
| Issue #2 API Key 向导 | 170942e |
| Issue #3 config setup 修复 | 1c86cc5 |
| `closeclaw chat`（TCP + LLM provider 集成） | 2470765 |
| `closeclaw stop -f`（强制关闭模式） | 5e82c75 |
| Permission Engine 用户维度（Subject::UserAndAgent + templates + Caller） | ebbec79 |
| 集成测试（cross-module interactions） | 86d942f |
| SIGTERM handler（graceful shutdown 触发） | fb7cc6b |
| Drain loop busy_count 检查 | 9967512 |
| Permission Engine bug 修复（Creator Rule + template expansion） | f99257c |
| Audit test isolation 修复 | 4e157b6 |
| Skill 列表精简（allowBundled 配置） | — |
| Workspace skill code-dev 创建 | — |
| docs/skills/ 重组为 docs/{operator,developer,skill-creator}/ | — |
| 多层级 Agent 架构设计文档 | — |

---

*由 Vibe虾 🦐 维护 | 最后更新: 2026-03-24T06:32*
