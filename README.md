# CloseClaw

> 轻量级、规则驱动的多 agent 执行框架

## 项目状态

🟡 **架构设计中** — 参见 [SPEC.md](./SPEC.md)

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
# 编译
cargo build --release

# 运行
cargo run --release
```

## 目录结构

```
closeclaw/
├── src/
│   ├── gateway/      # 网关模块
│   ├── agent/        # agent 运行时
│   ├── permission/   # 权限引擎（核心）
│   ├── config/       # 配置系统
│   ├── im/           # IM 适配器
│   ├── skills/       # 内置 skills
│   └── llm/          # LLM 接口抽象
└── tests/            # 测试
```

## License

TBD
