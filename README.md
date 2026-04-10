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

## 开发指南

- [SPEC_CONVENTION.md](SPEC_CONVENTION.md) — SPEC 编写规范（模块规格书怎么写）
- `src/<模块>/SPEC.md` — 各模块规格书（陆续建立中）
- [docs/](docs/) — 项目级文档、开发者指南

## License

MIT
