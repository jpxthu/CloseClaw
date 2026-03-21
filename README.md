# CloseClaw

> 轻量级、规则驱动的多 agent 执行框架

## 项目状态

🟢 **开发中** — Phase 8: 测试覆盖率完成

**测试：107 tests 全部通过**

| Phase | 状态 | 分支 |
|-------|------|------|
| Phase 1: 架构设计 | ✅ 完成 | master |
| Phase 2: Permission Engine | ✅ 完成 | master |
| Phase 3: Config System + Agent Runtime | ✅ 完成 | master |
| Phase 4: Gateway + IM | ✅ 完成 | master |
| Phase 5: Skill System | ✅ 完成 | master |
| Phase 6: LLM 接口抽象 | ✅ 完成 | master |
| Phase 7: CLI + 主程序 | ✅ 完成 | master |

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
cargo build

# 运行测试
cargo test

# 运行
cargo run
```

## 开发

```bash
# 查看所有分支
git branch -a

# 切换到最新开发分支
git checkout phase4-gateway-im

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

## 设计文档

- [SPEC.md](./SPEC.md) - 完整架构设计
- [docs/permission/](docs/permission/) - 权限系统文档

## License

MIT
