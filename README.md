# CloseClaw

> 轻量级、规则驱动的多 agent 执行框架

## 开发纪律：Spec-first

**所有代码实现完全依据 `docs/` 下的 Spec 文档，文档与代码功能模块一一对应。**

这是 CloseClaw 的核心开发纪律：
- 需求来了 → 先写 Spec，Spec 评审通过后再写代码
- 代码必须与 Spec 一致，不允许 Spec 没有就去写代码
- Spec 变更必须经过评审，不能单方面改代码不改文档

这条纪律保证了 agent 行为的可预测性和可审计性，是 CloseClaw 区别于其他 agent 框架的关键。

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

## 快速开始

```bash
# 克隆
git clone git@github.com:jpxthu/CloseClaw.git
cd CloseClaw

# 编译
cargo build                  # Debug build (fast)

# 运行测试
cargo test

# Release build (for production)
cargo build --release

# 运行
cargo run
```

## 开发

```bash
# 查看所有分支
git branch -a

# 切换到主分支（所有 Phase 1-7 功能已合并）
git checkout master

# 运行所有测试
cargo test

# 运行带日志
RUST_LOG=debug cargo test
```

## 目录结构

```
closeclaw/
├── src/
│   ├── gateway/      # 网关模块（消息路由、Session 管理）
│   ├── agent/        # agent 运行时（状态机、进程管理）
│   ├── permission/   # 权限引擎（核心）
│   ├── config/       # 配置系统（热重载、备份回滚）
│   ├── im/           # IM 适配器
│   ├── skills/       # 内置 skills
│   └── llm/          # LLM 接口抽象
├── tests/            # 集成测试
└── docs/             # 设计文档
```

## Spec 文档

**Spec-first 开发模式**：`docs/` 下的目录结构与 `src/` 完全对应，每个模块的 Spec 文档（设计决策、API 约定、行为规范）放在对应目录下。

- **目录对应**：`docs/<模块>/` ↔ `src/<模块>/`，一一对应
- **内容约束**：文档描述"模块做什么、做到什么程度"，代码依据文档实现
- **拆分原则**：文档拆分与代码拆分独立，各自按职责划分，不要求一一文件对应
- **变更规则**：文档变更必须评审，不能单方面改代码不改文档

现有 Spec 文档：

- [docs/developer/](docs/developer/README.md) - 开发指南
- [docs/permission/](docs/permission/OVERVIEW.md) - 权限引擎
- [docs/agent/](docs/agent/README.md) - Agent 模块
- [docs/gateway/](docs/gateway/README.md) - 网关
- [docs/llm/](docs/llm/README.md) - LLM 接口
- [docs/config/](docs/config/README.md) - 配置系统
- [docs/operator/](docs/operator/SKILL.md) - 运维指南

## License

MIT
