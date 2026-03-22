---
name: closeclaw-developer
description: |
  CloseClaw 框架开发指南。Use when: (1) developing or contributing to CloseClaw, (2) understanding its architecture or design decisions, (3) implementing new phases (Phase 5-11), (4) setting up the project, (5) configuring agents, permissions, or IM adapters, (6) onboarding to the codebase.
---

# CloseClaw Developer Guide

> 轻量级、规则驱动的多 agent 执行框架。用于 CloseClaw 自身的开发。

## Quick Start

```bash
# 1. 克隆 + 构建
git clone https://github.com/jpxthu/closeclaw.git
cd closeclaw && cargo build

# 2. 配置第一个 agent
closeclaw config setup

# 3. 启动 daemon
closeclaw run --config-dir ./configs
```

详细文档见 `docs/SETUP.md`（环境配置）和 `docs/WORKFLOW.md`（开发流程）。

---

## 项目状态

🟢 **Phase 1-7 完成**，Phase 8-11 待实现。

| Phase | 内容 | 状态 |
|-------|------|------|
| 1-4 | PE / Config / Agent Runtime / Gateway + Feishu | ✅ |
| 5 | Skill System | ❌ |
| 6 | 测试覆盖率（107 tests） | ✅ |
| 7 | CLI + Daemon + Graceful Shutdown + `closeclaw chat` | 进行中 |
| 8 | Inter-agent 通信 | ❌ |
| 9 | 配置热重载 | ❌ |
| 10 | 多 IM 适配器 | ❌ |
| 11 | 日志与审计系统 | ❌ |

---

## 核心设计原则

1. **规则优先**：所有权限由规则引擎裁定，模型输出不得绕过规则
2. **编译时安全**：PE 规则编译时绑定，运行时只读、不可被篡改
3. **最小权限**：每只 agent 只拥有完成其任务所需的最小权限集
4. **模块化可插拔**：IM adapter、skill、存储后端均可替换
5. **可观测性**：所有权限判断、agent 操作、错误均有日志和审计记录
6. **配置即代码友好**：JSON 配置，支持版本管理
7. **并发友好**：权限判断异步非阻塞，支持高并发场景
8. **配置容错**：配置错误不会导致整个服务崩溃，支持自动回滚
9. **跨平台**：Linux/Windows 双平台支持
10. **LLM 接口抽象**：trait 设计，支持 OpenAI/Anthropic/MiniMax 多后端

---

## 架构概览

```
CloseClaw
├── Gateway        ← IM 协议适配、消息路由
├── Agent Runtime  ← 每只 agent 独立进程
├── Permission Engine  ← 独立进程，规则评估 + OS 沙盒
├── Skill System   ← 可插拔 skill，file_ops / git_ops 等
├── Config System  ← JSON 模块分离，热重载，容错回滚
└── CLI Tool       ← closeclaw <command>
```

详细组件说明见 [references/architecture.md](references/architecture.md)。

---

## 权限规则速查

```json
{
  "version": "1.0",
  "rules": [
    {
      "name": "dev-agent-file-read",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        { "type": "file", "operation": "read", "paths": ["/home/admin/code/**"] }
      ]
    }
  ],
  "defaults": { "file": "deny", "command": "deny", "network": "deny" }
}
```

完整规则格式、评估逻辑、PE 接口定义见 `docs/permission/RULES.md`。

---

## Agent 模型速查

- 每只 agent = 独立 Rust 进程 + LLM 会话 + skill 集合 + 权限配置
- **层级**：Root → Child（继承 + 可覆盖收紧）
- **通信**：不共享内存，经 Registry 中转 + PE inter_agent 规则审查
- **配置**：agents.json 定义

Agent 组件 API 详见 `docs/agent/README.md`。详细层级继承、inter-agent 通信协议见 [references/agent-model.md](references/agent-model.md)。

---

## CLI 命令

```bash
closeclaw agent list              # 列出所有 agent
closeclaw agent create <name>    # 创建新 agent
closeclaw config validate <file> # 验证配置文件
closeclaw rule check <rule>      # 检查权限规则
closeclaw skill list             # 列出已安装 skill
closeclaw skill install <name>   # 安装 skill
closeclaw chat                   # 本地 CLI 直连 daemon（REPL 模式）
closeclaw chat -m "hello"       # 单消息模式
closeclaw run --config-dir ./configs  # 启动 daemon
closeclaw stop                   # 停止 daemon
```

完整命令列表见 `docs/cli/README.md`。

---

## 目录结构

```
src/
├── main.rs                # 入口
├── lib.rs                 # 库入口
├── cli/                   # CLI 命令
├── gateway/              # 网关（消息路由）
├── agent/               # Agent Runtime + Registry
├── permission/           # Permission Engine
│   ├── engine.rs        # 规则评估器
│   ├── sandbox/         # OS 沙盒（seccomp / landlock）
│   └── actions/         # 操作类型
├── config/               # 配置系统
├── im/                  # IM 适配器（飞书已实现）
├── skills/              # 内置 skill（coding_agent / skill_creator）
├── llm/                 # LLM 接口抽象（OpenAI / Anthropic / MiniMax）
└── daemon/              # Daemon 框架 + Graceful Shutdown
```

---

## 开发流程

1. 从 issue 创建 branch（`feat/xxx` / `fix/xxx`）
2. 开发 + 写测试
3. 确保 `cargo test` 全过 + `cargo clippy` 无警告
4. 更新对应文档
5. 开 Pull Request，描述改了什么、为什么
6. Review 合并后 close issue

详细流程见 `docs/GITHUB_WORKFLOW.md`。

---

## 详细参考文档

| 文档 | 内容 |
|------|------|
| [references/architecture.md](references/architecture.md) | 核心设计原则 + 组件详解 + 源码目录结构 |
| [references/agent-model.md](references/agent-model.md) | Agent 模型 + 层级继承 + inter-agent 通信协议 |
| [references/risk-issues.md](references/risk-issues.md) | 风险表 + 团队角色 + 术语表 + OpenClaw 参考 |
| `docs/permission/RULES.md` | 完整权限规则格式 + 评估逻辑 + PE 接口 |
| `docs/permission/OVERVIEW.md` | 权限系统设计概述 |
| `docs/permission/API.md` | Permission Engine API 参考 |
| `docs/config/README.md` | 配置系统 + 热重载 + 容错机制 |
| `docs/agent/README.md` | Agent 组件 API（Registry / Process / State） |
| `docs/gateway/README.md` | Gateway + IMAdapter API |
| `docs/llm/README.md` | LLM provider 接口 |
| `docs/daemon-graceful-shutdown.md` | Daemon Graceful Shutdown 详解 |
| `git.md` | Git 操作规范 |
| `cargo.md` | Cargo 命令参考 |
| `code-style.md` | Rust 代码规范 |
| `commit-style.md` | Commit 格式规范 |

## 其他文档

- `docs/SETUP.md` — 环境配置指南
- `docs/WORKFLOW.md` — 开发流程
- `docs/GITHUB_WORKFLOW.md` — GitHub Issues 驱动流程
- `QUICKSTART.md` — 快速上手

---

*由 Vibe虾 🦐 维护*
