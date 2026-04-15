# CloseClaw

> 轻量级、规则驱动的多 agent 执行框架

## Spec 规格书

CloseClaw 使用 **Spec-first** 开发模式：

- **规格书位置**：`src/<模块名>/SPEC.md`（每个模块一个规格书）
- **规格书内容**：模块的精确功能说明（接口、数据结构、行为规范），不是开发步骤
- **编写规范**：见 [SPEC_CONVENTION.md](SPEC_CONVENTION.md)
- **当前状态**：大部分模块规格书缺失，正在逐步建立（见 SPEC_CONVENTION.md）

> **Spec-first 开发纪律**：需求来了 → developer 先写 SPEC → braino review 通过后再开发 → 代码与 SPEC 保持镜像一致

## 核心特性

- **规则驱动的权限引擎**：每只 agent 的权限在代码层面编译进沙盒，不可绕过
- **模块化架构**：Gateway、Agent Runtime、Permission Engine、IM Adapter、Skill System 独立可插拔
- **多 IM 后端支持**：飞书、Telegram、Discord 等即时通讯工具可插拔接入
- **清晰的配置系统**：JSON 格式配置，模块分离，变更可追踪
- **可见性与可追溯**：agent 动作、权限判断过程、agent 间通信均可观测

## 技术栈

- **语言**：Rust
- **并发运行时**：Tokio
- **IPC**：Unix Domain Socket + async channels
- **OS 安全层**：seccomp + landlock
- **构建工具**：Cargo

## 目录结构

> **维护规则**：模块结构或功能变更时，必须同步更新本表。新增模块也必须在此注册。

| 文件 | 描述 |
|------|------|
| `src/agent/SPEC.md` | Agent 配置、prompt 构造、能力调度 |
| `src/card/SPEC.md` | 卡片消息渲染与交互处理 |
| `src/chat/SPEC.md` | 聊天会话管理、上下文构建 |
| `src/config/SPEC.md` | 配置加载、校验、热点更新 |
| `src/gateway/SPEC.md` | 网关协议接入（IM 适配层） |
| `src/im/SPEC.md` | IM 消息接收与发送、事件处理 |
| `src/llm/SPEC.md` | LLM 接口抽象、多模型支持 |
| `src/mode/SPEC.md` | 运行模式（CLI/Gateway/Daemon） |
| `src/permission/SPEC.md` | 权限校验与访问控制 |
| `src/platform/SPEC.md` | 平台层抽象（飞书/Discord/Signal…） |
| `src/session/SPEC.md` | Session 存储与生命周期管理 |
| `src/skills/SPEC.md` | Skill 加载、注册、调度 |
| `src/system_prompt/SPEC.md` | System Prompt 分段渲染 |

## 启动

```bash
cargo build && cargo test
cargo run
```
