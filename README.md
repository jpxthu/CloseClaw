# CloseClaw

轻量级、规则驱动的多 agent 执行框架。Rust + Tokio。

```bash
cargo build && cargo test
```

## 开发纪律

### Spec-first

- 每个模块有规格书：`src/<模块名>/SPEC.md`
- SPEC 描述"系统现在是什么"，不是开发步骤
- 代码改了 SPEC 必须同步，SPEC 改了代码也必须同步
- 编写规范见 [SPEC_CONVENTION.md](SPEC_CONVENTION.md)

### 代码硬限制

| 指标 | 上限 |
|------|------|
| 文件行数 | 500 |
| 单行宽度 | 100 字符 |
| 函数体行数 | 50 |
| 函数参数 | 6 |
| 模块嵌套深度 | 3 层 |
| impl 块行数 | 100 |
| enum 变体数 | 20 |
| 嵌套 match/if | 3 层 |
| unsafe 块 | 0（除非有注释说明） |

### Commit 格式

```
<type>(<scope>): <description>

Source: <issue #N | CI | user>
Type: <type>
```

`Source:` 和 `Type:` footer 是 **强制要求**，CI 会校验。详见 [docs/developer/commit-style.md](docs/developer/commit-style.md)。

### 编码规范

- 命名：类型 `UpperCamelCase`，函数/变量 `snake_case`，常量 `SCREAMING_SNAKE_CASE`
- 布尔变量加前缀 `is_` / `has_` / `can_` / `should_`
- 错误：`thiserror` 定义错误类型，`anyhow` 包装上下文，`?` 传播
- 测试：单元测试同文件 `#[cfg(test)]`，集成测试放 `tests/`
- 完整规范见 [docs/developer/code-style.md](docs/developer/code-style.md)

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
| `src/session/` | Session 存储与生命周期管理 | [SPEC](src/session/SPEC.md) |
| `src/skills/` | Skill 加载、注册、调度 | [SPEC](src/skills/SPEC.md) |
| `src/system_prompt/` | System Prompt 分段渲染 | [SPEC](src/system_prompt/SPEC.md) |

## 关键文件索引

| 需要了解 | 去看 |
|----------|------|
| 环境搭建、构建命令 | [docs/SETUP.md](docs/SETUP.md) |
| 编码规范、硬限制详情 | [docs/developer/code-style.md](docs/developer/code-style.md) |
| Commit 格式、CI 门禁 | [docs/developer/commit-style.md](docs/developer/commit-style.md) |
| Git 工作流 | [docs/developer/git-guide.md](docs/developer/git-guide.md) |
| Cargo 命令速查 | [docs/developer/cargo.md](docs/developer/cargo.md) |
| SPEC 编写规范 | [SPEC_CONVENTION.md](SPEC_CONVENTION.md) |
| Agent 模型与通信 | [docs/developer/references/agent-model.md](docs/developer/references/agent-model.md) |
| 风险项、术语表 | [docs/developer/references/risk-issues.md](docs/developer/references/risk-issues.md) |
| 配置示例 | `configs/agents.json.example`、`configs/.env.example` |
