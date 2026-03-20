# SPEC.md — CloseClaw

> 轻量级、规则驱动的多 agent 执行框架

---

## 状态

🟡 **需求收集完成 / 架构设计中** — 待详细设计文档填充

---

## 1. 项目概述

### 1.1 背景与动机

OpenClaw 的网关 + 多 agent 协作模式非常出色，但在实际使用中暴露了以下问题：

| 痛点 | 描述 |
|------|------|
| 臃肿冗余 | 启动慢，功能耦合，修改一处可能影响全局 |
| 权限管理不透明 | 无法精细控制某只 agent 能执行什么命令、访问什么文件；现有方案依赖 prompt 约束，不可靠 |
| 配置集中风险 | 所有配置集中在一个文件，错了整个服务崩；变更难以追踪 |
| agent 间协作不可见 | agent 之间对话用户看不到，缺乏可观测性 |
| 配置即脆弱点 | 频繁变更配置时代码和配置混杂，缺乏模块化 |
| 可观测性不足 | 难以追踪 agent 的动作和决策过程 |

### 1.2 目标

**CloseClaw** 是一个轻量级的多 agent 执行框架，核心特点：

1. **规则驱动的权限引擎**：每只 agent 的权限在代码层面被编译进沙盒，不可绕过
2. **模块化架构**：各组件（网关、agent runtime、权限引擎、IM adapter、skill system）独立可插拔
3. **清晰的配置系统**：JSON 格式配置，模块分离，支持变更追踪
4. **多 IM 后端支持**：飞书、Telegram、Discord 等即时通讯工具可插拔接入
5. **可见性与可追溯**：agent 动作、权限判断过程、agent 间通信均可观测

### 1.3 与 OpenClaw 的关系

**继承：**
- 网关模式（消息路由、协议抽象）
- 多 agent 协作模型（带人设的 agent）
- Skills / 插件系统
- 通过对话实现配置和管理的能力
- IM 集成（Telegram、Discord 等已积累的能力）

**改进：**
- 臃肿 → 轻量，模块化，可按需加载
- 权限模糊 → 代码级显式规则，不可绕过
- 配置集中 → 模块化配置，变更可追踪
- agent 间不可见 → 通信记录可查
- 单一配置文件 → 配置分模块，支持热重载

---

## 2. 核心设计原则

1. **规则优先**：所有权限由规则引擎裁定，模型输出不得绕过规则
2. **编译时安全**：规则编译进二进制，运行时不可动态修改
3. **最小权限**：每只 agent 只拥有完成其任务所需的最小权限集
4. **模块化可插拔**：IM adapter、skill、存储后端均可替换
5. **可观测性**：所有权限判断、agent 操作、错误均有日志和审计记录
6. **配置即代码友好**：JSON 配置，支持版本管理
7. **并发友好**：权限判断异步非阻塞，支持高并发场景

---

## 3. 系统架构

### 3.1 整体架构图

```
┌─────────────────────────────────────────────────────────────┐
│                         CloseClaw                           │
│                                                             │
│  ┌──────────┐    ┌────────────────┐    ┌─────────────────┐  │
│  │ IM       │    │   Gateway      │    │  Agent Registry │  │
│  │ Adapters │◄──►│  (Router)      │◄──►│  & Lifecycle     │  │
│  │ (Feishu, │    │  - Protocol    │    │  - Spawn/Kill   │  │
│  │  Discord,│    │  - Auth        │    │  - Config       │  │
│  │  etc.)   │    │  - Rate Limit   │    │  - Inter-agent  │  │
│  └──────────┘    └────────────────┘    │    comm        │  │
│                                         └────────┬────────┘  │
│                                                  │          │
│            ┌─────────────────────────────────────┼────────┐ │
│            │                    Agent Runtime                  │ │
│            │  ┌─────────────┐  ┌─────────────┐  ┌─────────┐ │ │
│            │  │  Agent A    │  │  Agent B    │  │  ...    │ │ │
│            │  │  (LLM Capable)│ │  (LLM Capable)│ │         │ │ │
│            │  └──────┬──────┘  └──────┬──────┘  └────┬────┘ │ │
│            │         │                │               │      │ │
│            └─────────┼────────────────┼───────────────┼──────┘ │
│                      │                │               │        │
│            ┌─────────▼────────────────▼───────────────▼────┐  │
│            │              Permission Engine (PE)            │  │
│            │  ┌─────────────────────────────────────────┐  │  │
│            │  │  Rule DB  │  Evaluator  │  Executor     │  │  │
│            │  │  (static) │  (async)     │  (seccomp)    │  │  │
│            │  └─────────────────────────────────────────┘  │  │
│            │  Runs as separate OS process                  │  │
│            └──────────────────────────────────────────────┘  │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │              Skill System (pluggable)                 │  │
│  │  file_ops │ git_ops │ search │ calendar │ ...        │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │              Config System (module-based)             │  │
│  │  agents.json │ permissions.json │ im.json │ skills.json│ │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 核心组件

#### Gateway（网关）
- 职责：IM 协议适配、消息路由、认证、限流
- 协议抽象：所有 IM 消息统一转换为内部 `Message` 结构
- 可插拔 IM Adapter：同一 Gateway 接口可挂载多个 IM 后端

#### Agent Registry
- 职责：管理所有 agent 的生命周期（创建、销毁、查询）
- 维护 agent 的配置、权限上下文、当前状态
- 支持层级结构：parent agent → child agent，权限可继承可覆盖

#### Agent Runtime
- 每只 agent 运行在一个独立线程或进程中（可配置）
- agent 持有 LLM 接口凭证、skill 实例、本地工具
- agent 的所有操作（文件、命令、网络）必须通过 Permission Engine

#### Permission Engine（权限引擎）— 核心创新
- 架构：**独立 OS 进程**，不与 agent 共用地址空间
- 规则：编译时加载，运行期只读，无法被 agent 篡改
- 接口：Unix Domain Socket + async channel，接收结构化请求
- 规则粒度：
  - **文件系统**：允许/禁止访问的具体路径（支持 glob），读写执行分类
  - **命令**：允许执行的命令白名单，参数约束
  - **网络**：允许访问的域名/IP、端口范围
  - **工具调用**：允许使用的 skill 方法
  - **inter-agent**：允许通信的 agent 列表
- OS 层隔离：`seccomp` + `landlock` 锁定 syscalls
- 性能：规则查表 O(1)，异步非阻塞，不成为并发瓶颈

#### Skill System
- 与 OpenClaw skill 机制兼容
- 可插拔：skill 注册后 agent 可声明依赖
- skill 同样受 Permission Engine 管辖

#### Config System
- 模块化 JSON 配置文件，变更可追踪（git）
- 配置分块：
  - `agents.json` — agent 定义、人设、parent/child 关系
  - `permissions.json` — 权限规则（PE 规则）
  - `im.json` — IM adapter 配置
  - `skills.json` — skill 注册和启用
  - `gateway.json` — 网关配置
- 支持热重载（部分配置）

---

## 4. 权限规则设计

### 4.1 规则格式（JSON）

```json
{
  "version": "1.0",
  "rules": [
    {
      "name": "dev-agent-file-read",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "file",
          "operation": "read",
          "paths": ["/home/admin/code/**"]
        },
        {
          "type": "file",
          "operation": "write",
          "paths": ["/home/admin/code/closeclaw/src/**"]
        }
      ]
    },
    {
      "name": "dev-agent-git",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "allowed": ["status", "log", "diff", "add", "commit", "push", "pull"] }
        }
      ]
    },
    {
      "name": "dev-agent-forbidden-git-reset",
      "subject": { "agent": "dev-agent-01" },
      "effect": "deny",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "blocked": ["reset", "rebase", "push", "--force"] }
        }
      ]
    },
    {
      "name": "readonly-agent",
      "subject": { "agent": "readonly-*", "match": "glob" },
      "effect": "allow",
      "actions": [
        { "type": "file", "operation": "read", "paths": ["**"] }
      ]
    }
  ],
  "defaults": {
    "file": "deny",
    "command": "deny",
    "network": "deny",
    "inter_agent": "deny"
  }
}
```

### 4.2 规则评估逻辑

1. 请求到达 PE → 解析 action 类型
2. 匹配 `subject`（精确或 glob）
3. 按规则 `name` 顺序（或优先级）评估 `effect`
4. `deny` 优先于 `allow`（类似 AWS IAM）
5. 未匹配任何规则 → `defaults` 裁定
6. 返回结果（允许/拒绝 + 原因）

### 4.3 Permission Engine 接口

```rust
// 请求格式（通过 Unix socket 发送）
pub enum PermissionRequest {
    FileOp { agent: String, path: String, op: FileOp },
    CommandExec { agent: String, cmd: String, args: Vec<String> },
    NetOp { agent: String, host: String, port: u16 },
    ToolCall { agent: String, skill: String, method: String },
    InterAgentMsg { from: String, to: String },
}

// 响应格式
pub enum PermissionResponse {
    Allowed { token: String },  // 带操作令牌，有效期短
    Denied { reason: String, rule: String },
}
```

---

## 5. Agent 模型定义

### 5.1 什么是"一只 agent"

- 独立的 Rust 线程或进程（可配置）
- 持有一个 LLM 会话（可接入多个 LLM 提供者）
- 拥有自己的人设（system prompt）、skill 集合、权限配置
- 通过 Permission Engine 与系统资源交互
- 通过 Agent Registry 与其他 agent 通信

### 5.2 层级与继承

```
         ┌──────────────────────────────────────┐
         │         Root Agent (你)               │
         │  权限：所有（受 defaults 限制）        │
         └──────────────┬───────────────────────┘
                        │ parent
          ┌─────────────┼─────────────┐
          ▼             ▼             ▼
   ┌──────────┐ ┌──────────┐ ┌──────────┐
   │ dev-01   │ │ dev-02   │ │ qa-01    │
   │ 继承 root │ │ 继承 root │ │ 继承 root │
   │ + 额外限制 │ │ + 额外限制 │ │ + 额外限制 │
   └──────────┘ └──────────┘ └──────────┘
```

- **继承**：child agent 默认继承 parent 的所有权限
- **覆盖**：child 可在父权限基础上添加额外限制（收紧，不可放松）
- **平级**：同层级 agent 默认无法互相通信（由 `inter_agent` 规则控制）

### 5.3 Inter-Agent 通信

- agent 之间不共享内存，通过结构化消息通信
- 消息经过 Permission Engine 的 `inter_agent` 规则审查
- 支持：request/response、event/notification、bidirectional stream
- 所有消息有 sender/receiver/timestamp/content，可记录可审计

### 5.4 Agent 配置示例（agents.json 片段）

```json
{
  "agents": [
    {
      "id": "dev-01",
      "name": "开发助手",
      "parent": "root",
      "persona": "你是 CloseClaw 的开发助手，负责编写高质量 Rust 代码...",
      "skills": ["file_ops", "git_ops", "rust_analyzer"],
      "llm": {
        "provider": "openai",
        "model": "gpt-4o"
      },
      "config": {
        "max_concurrent_ops": 4,
        "timeout_seconds": 300
      }
    }
  ]
}
```

---

## 6. 功能范围

### 6.1 MVP（Phase 2）

- [x] Permission Engine 核心（文件、命令白名单）
- [x] 规则加载和评估
- [x] Agent Runtime 骨架（单 agent 可运行）
- [x] Gateway 骨架（单 IM 适配器，可选飞书）
- [x] 配置系统骨架（JSON 模块分离）

### 6.2 V1（Phase 3）

- [ ] 完整的 inter-agent 通信机制
- [ ] 多 IM 适配器框架（Telegram、Discord）
- [ ] Skill 系统基础实现
- [ ] 配置热重载
- [ ] 日志与审计系统
- [ ] 基础 CI/CD

### 6.3 未来

- [ ] Web UI / Dashboard
- [ ] 分布式 agent 支持
- [ ] 持久化存储后端（SQLite/Postgres）
- [ ] 云端部署支持
- [ ] VS Code / JetBrains 插件

---

## 7. 技术选型

| 维度 | 选项 | 决策 |
|------|------|------|
| **语言** | Rust | ✅ 编译安全、性能、内存管控 |
| **并发运行时** | Tokio | ✅ 成熟 async actor 框架 |
| **IPC** | Unix Domain Socket + tokio channels | ✅ 高效、安全（与 PE 进程通信） |
| **OS 安全层** | seccomp + landlock | ✅ Linux 原生 syscalls 限制 |
| **配置格式** | JSON | ✅ 人类可读、版本管理友好 |
| **序列化** | serde + JSON | ✅ |
| **日志** | tracing + tracing-subscriber | ✅ 结构化日志 |
| **测试** | cargo test + proptest + cargo-fuzz | ✅ 单元/属性/模糊测试 |
| **LLM 接口** | 抽象 trait，可接入 OpenAI/Anthropic/本地模型 | ✅ |
| **IM 适配** | 插件化 trait，每个 IM 一个实现 | ✅ |
| **构建工具** | Cargo | ✅ |

---

## 8. 目录结构

```
closeclaw/
├── Cargo.toml
├── SPEC.md
├── README.md
├── src/
│   ├── main.rs              # 入口
│   ├── lib.rs               # 库入口
│   ├── gateway/             # 网关模块
│   │   ├── mod.rs
│   │   ├── router.rs        # 消息路由
│   │   └── protocol.rs      # 统一协议
│   ├── agent/                # agent 运行时
│   │   ├── mod.rs
│   │   ├── runtime.rs       # agent 生命周期
│   │   ├── registry.rs      # agent 注册表
│   │   └── interop.rs       # agent 间通信
│   ├── permission/          # 权限引擎（核心）
│   │   ├── mod.rs
│   │   ├── engine.rs        # 规则评估器
│   │   ├── rules/           # 规则定义
│   │   ├── actions/         # 操作类型（file/command/network）
│   │   └── sandbox/         # OS 层沙盒（seccomp）
│   ├── config/              # 配置系统
│   │   ├── mod.rs
│   │   ├── agents.rs        # agents.json
│   │   ├── permissions.rs   # permissions.json
│   │   └── im.rs            # im.json
│   ├── im/                  # IM 适配器
│   │   ├── mod.rs
│   │   ├── adapter.rs       # 适配器 trait
│   │   └── feishu.rs        # 飞书实现
│   ├── skills/              # 内置 skills
│   │   ├── mod.rs
│   │   ├── file_ops.rs
│   │   └── git_ops.rs
│   └── llm/                 # LLM 接口抽象
│       ├── mod.rs
│       └── client.rs
└── tests/
    ├── integration/
    └── permission/
```

---

## 9. 风险与开放问题

| 问题 | 状态 | 说明 |
|------|------|------|
| landlock 对容器环境要求 | 待确认 | 需内核 5.13+，云服务器兼容性需测 |
| LLM 接口标准化 | 待定 | 多 provider 的接口抽象如何设计 |
| agent 通信协议 | 草案 | 具体 wire format 待定义 |
| seccomp 规则粒度 | 待定 | 过严影响功能，过松失去保护 |
| 配置热重载原子性 | 待定 | 多模块配置变更的原子更新 |

---

## 10. 团队角色定义

| 角色 | 职责 |
|------|------|
| **主 agent**（我） | 统筹全局、决策拍板、对外沟通、最终审核交付 |
| **PM agent** | 需求分析、SPEC.md 撰写和维护、设计文档 review |
| **Dev agent × N** | 并行开发各模块代码 |
| **QA agent** | 对需求和设计文档找茬提问、写足够的测试用例、验证测试覆盖率 |
| **Code Reviewer** | 交叉 code review、安全审计、确保实现符合设计 |

**三角制约关系：**
```
Dev agent ←→ Code Reviewer
    ↑              ↑
    └──  QA agent ──┘
```

## 11. 开发阶段计划

| Phase | 内容 | 产出 | Agent 分工 |
|-------|------|------|-----------|
| **Phase 1** | 需求确认 + 架构设计 | 完善 SPEC.md | 全部由主 agent 完成 |
| **Phase 2** | Permission Engine 核心实现 | `src/permission/` 代码 | Dev agent A：PE engine + sandbox |
| **Phase 2b** | 配置系统实现 | `src/config/` 代码 | Dev agent B：配置加载和验证 |
| **Phase 2c** | Agent Runtime 骨架 | `src/agent/` 骨架代码 | Dev agent C：runtime 骨架 |
| **Phase 3** | Gateway + IM 适配器 | `src/gateway/` + `src/im/` | Dev agent D：gateway + 飞书 adapter |
| **Phase 4** | Skill 系统基础实现 | `src/skills/` | Dev agent A：file_ops skill |
| **Phase 5** | 集成测试 + 修正 | 可运行的最小闭环 | QA agent：测试 + coverage report |
| **Phase 6** | 代码审核 + 安全审计 | 审核报告 | Code Reviewer：交叉 review + PE 重点审计 |
| **Phase 7** | 文档完善 + 交付 | 最终版本 | 主 agent 完成 |

**开发过程中的监督机制：**
- Dev agent 之间交叉 code review（防止自己写自己审）
- QA agent 在 Phase 1 结束后就开始 review 设计文档，提前找茬
- PE 代码必须经过 Code Reviewer + QA agent 双重审核
- 测试覆盖率作为合并标准（目标：核心模块 > 80%）

---

## 附录 A：术语表

| 术语 | 定义 |
|------|------|
| PE | Permission Engine，权限引擎，独立进程运行，不可被篡改 |
| Agent | 具有 LLM 能力的执行单元，可理解指令并通过工具行动 |
| Skill | 封装好的工具能力，可被 agent 调用 |
| IM Adapter | 即时通讯后端适配器，负责协议转换 |
| Rule | 权限规则，声明式定义某 agent 能做什么 |
| seccomp | Linux 内核安全机制，锁定进程可使用的系统调用（syscall），防止提权 |
| landlock | Linux 内核安全机制，细粒度文件系统权限控制，限制进程对指定目录/文件的访问 |
| O(1) 查表 | 规则评估复杂度为常数时间，与规则数量无关 |
| 异步评估 | 权限判断不阻塞 agent 并发执行，PE 通过 async channel 接收/响应请求 |

---

*最后更新：Phase 1 需求确认完成后，本文档由需求文档升级为架构设计文档。*
