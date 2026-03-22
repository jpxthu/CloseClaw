# Architecture — 核心组件详解

## 整体架构图

```
┌──────────────────────────────────────────────────────────────────┐
│                         CloseClaw                                  │
│  ┌──────────┐    ┌────────────────┐    ┌─────────────────┐      │
│  │ IM       │◄──►│   Gateway      │◄──►│  Agent Registry │      │
│  │ Adapters │    │  - Protocol    │    │  - Spawn/Kill   │      │
│  └──────────┘    └────────────────┘    └─────────────────┘      │
│                  ┌──────────────────────────────┐               │
│                  │       Agent Runtime           │               │
│                  │  ┌──────────┐  ┌──────────┐ │               │
│                  │  │ Agent A  │  │ Agent B  │ │               │
│                  │  └──────────┘  └──────────┘ │               │
│                  └──────────────────────────────┘               │
│                  ┌──────────────────────────────┐               │
│                  │   Permission Engine (PE)     │               │
│                  │  独立 OS 进程，独立沙盒        │               │
│                  └──────────────────────────────┘               │
│  ┌──────────────────────┐  ┌──────────────────────────┐        │
│  │  Skill System        │  │  Config System           │        │
│  │  (pluggable)         │  │  (hot-reloadable)        │        │
│  └──────────────────────┘  └──────────────────────────┘        │
└──────────────────────────────────────────────────────────────────┘
```

## 核心组件

| 组件 | 职责 |
|------|------|
| **Gateway** | IM 协议适配、消息路由、认证、限流 |
| **Agent Registry** | 管理 agent 生命周期（创建、销毁、查询） |
| **Agent Runtime** | 每只 agent 独立进程，持 LLM + skill + 工具 |
| **Permission Engine** | 独立进程，规则评估 + OS 沙盒（seccomp/landlock） |
| **Scheduler** | Cron 定时任务、Heartbeat 心跳、Idle 超时管理 |
| **Skill System** | 可插拔 skill，file_ops/git_ops 等 |
| **Config System** | JSON 模块分离，热重载，容错回滚 |
| **CLI Tool** | `closeclaw <command>` |

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
