# CloseClaw

轻量级、规则驱动的多 agent 执行框架。Rust + Tokio。

```bash
cargo build && cargo test
```

## 模块地图

| 目录 | 功能 | SPEC |
|------|------|------|
| `src/agent/` | Agent 配置、prompt 构造、能力调度 | [SPEC](src/agent/SPEC.md) |
| `src/audit/` | 操作审计日志、事件记录与查询 | [SPEC](src/audit/SPEC.md) |
| `src/card/` | 卡片消息渲染与交互处理 | [SPEC](src/card/SPEC.md) |
| `src/chat/` | 聊天会话管理、上下文构建 | [SPEC](src/chat/SPEC.md) |
| `src/cli/` | 命令行启动、交互模式、参数解析 | [SPEC](src/cli/SPEC.md) |
| `src/config/` | 配置加载、校验、热点更新 | [SPEC](src/config/SPEC.md) |
| `src/daemon/` | Daemon 进程管理、信号处理、优雅关闭 | [SPEC](src/daemon/SPEC.md) |
| `src/gateway/` | 网关协议接入（IM 适配层） | [SPEC](src/gateway/SPEC.md) |
| `src/im/` | IM 消息接收与发送、事件处理 | [SPEC](src/im/SPEC.md) |
| `src/llm/` | LLM 接口抽象、多模型支持 | [SPEC](src/llm/SPEC.md) |
| `src/mode/` | 运行模式（CLI/Gateway/Daemon） | [SPEC](src/mode/SPEC.md) |
| `src/permission/` | 权限校验与访问控制 | [SPEC](src/permission/SPEC.md) |
| `src/platform/` | 平台层抽象（飞书/Discord/Signal…） | [SPEC](src/platform/SPEC.md) |
| `src/processor_chain/` | 消息处理链（入站/出站） | [SPEC](src/processor_chain/SPEC.md) |
| `src/renderer/` | 渲染层（Markdown → 卡片） | [SPEC](src/renderer/SPEC.md) |
| `src/session/` | Session 存储与生命周期管理 | [SPEC](src/session/SPEC.md) |
| `src/skills/` | Skill 加载、注册、调度 | [SPEC](src/skills/SPEC.md) |
| `src/system_prompt/` | System Prompt 分段渲染 | [SPEC](src/system_prompt/SPEC.md) |
| `src/tools/` | Tool 注册与调用管理 | [SPEC](src/tools/SPEC.md) |

> 部分模块的 SPEC.md 尚在编写中，见 [docs/design/](docs/design/README.md)。

## 关键链接

| 需要了解 | 去看 |
|----------|------|
| 开发规范（编码、测试、Git、PR） | [CONTRIBUTING.md](CONTRIBUTING.md) |
| 模块设计文档 | [docs/design/](docs/design/README.md) |
| 环境搭建 | [docs/SETUP.md](docs/SETUP.md) |
| SPEC 编写规范 | [SPEC_CONVENTION.md](SPEC_CONVENTION.md) |
