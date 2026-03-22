# Architecture — 核心设计原则与组件详解

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

## 整体架构图

```
┌──────────────────────────────────────────────────────────────────┐
│                         CloseClaw                                  │
│  ┌──────────┐    ┌────────────────┐    ┌─────────────────┐      │
│  │ IM       │◄──►│   Gateway      │◄──►│  Agent Registry │      │
│  │ Adapters │    │  - Protocol    │    │  - Spawn/Kill   │      │
│  │ 飞书     │    │  - Auth       │    │  - Config       │      │
│  │ 企业微信   │    │  - Rate Limit │    │  - Inter-agent  │      │
│  └──────────┘    └────────────────┘    └─────────────────┘      │
│                  ┌──────────────────────────────┐               │
│                  │       Agent Runtime           │               │
│                  │  ┌──────────┐  ┌──────────┐│               │
│                  │  │ Agent A  │  │ Agent B  ││               │
│                  │  │ (Process)│  │ (Process)││               │
│                  │  └──────────┘  └──────────┘│               │
│                  └──────────────────────────────┘               │
│                  ┌──────────────────────────────┐               │
│                  │   Permission Engine (PE)      │               │
│                  │  Rule DB │ Evaluator │ Exec  │               │
│                  │  (static) (async) (sandbox) │               │
│                  │  独立 OS 进程                 │               │
│                  └──────────────────────────────┘               │
│  ┌──────────────────────┐  ┌──────────────────────────┐        │
│  │  Skill System        │  │  Config System           │        │
│  │  file_ops│git_ops│...│  │  agents│perm│im│skills.json │        │
│  └──────────────────────┘  └──────────────────────────┘        │
│  ┌──────────────────────────────────────────────────────┐     │
│  │  CLI Tool (closeclaw <command>)                      │     │
│  └──────────────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────────────────┘
```

## 核心组件详解

| 组件 | 职责 |
|------|------|
| **Gateway** | IM 协议适配、消息路由、认证、限流。协议抽象：所有 IM 消息统一转换为内部 `Message` 结构 |
| **Agent Registry** | 管理 agent 生命周期（创建、销毁、查询）。支持 parent→child 层级，权限可继承 |
| **Agent Runtime** | 每只 agent 独立进程（可配置）。持 LLM 会话 + skill 实例 + 本地工具。所有操作必须经 PE |
| **Permission Engine** | 独立 OS 进程，规则评估 + OS 沙盒（seccomp/landlock）。接口：Unix Domain Socket + async channel |
| **Skill System** | 可插拔 skill，与 OpenClaw 兼容，coding_agent / skill_creator 等内置 skill |
| **Config System** | JSON 模块分离，`agents.json`/`permissions.json`/`im.json`/`skills.json` 分立，支持热重载和容错回滚 |
| **CLI Tool** | `closeclaw <command>`，包括 agent/config/rule/skill 子命令，以及 `closeclaw chat` 本地 CLI |
| **Daemon** | 长期运行进程框架，支持 Graceful Shutdown 状态机 |
| **Scheduler** | ⏳ 规划中 — Cron 定时任务、Heartbeat 心跳、Idle 超时管理、Lifecycle Hooks |

## Skill 安全 Review 机制（可选）

```json
{
  "skill_review": {
    "enabled": true,
    "security_expert_agent": "security-agent-01",
    "review_required_for": ["new", "modified"],
    "on_approve": "auto_enable",
    "on_reject": "block_and_notify",
    "on_timeout_hours": 48
  }
}
```

## 源码目录结构

```
src/
├── main.rs                  # 入口
├── lib.rs                   # 库入口
├── cli/                     # CLI 命令
│   ├── mod.rs
│   └── chat.rs            # closeclaw chat 子命令
├── gateway/                 # 网关：消息路由、协议抽象
│   ├── mod.rs
│   └── message.rs
├── agent/                   # Agent Runtime + Registry
│   ├── mod.rs
│   ├── process.rs          # OS 进程管理
│   └── registry.rs         # agent 注册表
├── permission/             # Permission Engine（核心）
│   ├── mod.rs
│   ├── engine.rs          # 规则评估器
│   ├── rules/             # 规则定义
│   ├── actions/           # 操作类型（file/command/network）
│   └── sandbox/           # OS 沙盒（seccomp / landlock）
├── config/                 # 配置系统
│   ├── mod.rs
│   ├── agents.rs           # agents.json
│   ├── providers/          # ConfigProvider trait 实现
│   ├── reload.rs         # 热重载
│   └── backup.rs          # 备份与回滚
├── im/                     # IM 适配器
│   ├── mod.rs
│   └── feishu.rs         # 飞书实现
├── skills/                 # 内置 skill
│   ├── mod.rs
│   ├── builtin.rs
│   ├── coding_agent.rs
│   ├── skill_creator.rs
│   └── registry.rs
├── llm/                    # LLM 接口抽象
│   ├── mod.rs
│   ├── openai.rs
│   ├── anthropic.rs
│   └── minimax.rs
├── chat/                   # TCP Chat Server（closeclaw chat 命令）
│   ├── mod.rs
│   ├── protocol.rs        # JSON 协议
│   ├── server.rs          # TCP 服务器
│   └── session.rs         # 会话管理
└── daemon/                 # Daemon 框架 + Graceful Shutdown
    ├── mod.rs
    └── shutdown.rs         # Graceful Shutdown 状态机
tests/
├── engine_test.rs
├── smoke_test.rs
└── comprehensive_tests.rs
```
