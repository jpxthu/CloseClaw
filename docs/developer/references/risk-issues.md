# Risk, Issues, Roles & Glossary

## 风险与开放问题

| 问题 | 状态 | 说明 |
|------|------|------|
| landlock 对容器环境要求 | 待确认 | 需内核 5.13+，云服务器兼容性需测 |
| Windows Sandbox 支持 | 待实现 | 需要单独研究实现方案 |
| seccomp 规则粒度 | 待定 | 过严影响功能，过松失去保护 |
| 配置热重载原子性 | Phase 9 | 多模块配置变更的原子更新 |
| agent 通信协议 | Phase 8 | 具体 wire format 待定义 |

## 未来项

- [ ] Web UI / Dashboard
- [ ] 分布式 agent 支持
- [ ] 持久化存储后端（SQLite/Postgres）
- [ ] 云端部署支持
- [ ] Windows 平台支持
- [ ] VS Code / JetBrains 插件

## 团队角色定义

| 角色 | 职责 |
|------|------|
| **主 agent** | 统筹全局、决策拍板、对外沟通、最终审核交付 |
| **PM agent** | 需求分析、SPEC.md 撰写和维护、设计文档 review |
| **Dev agent × N** | 并行开发各模块代码 |
| **QA agent** | 对需求和设计文档找茬提问，写测试用例、验证覆盖率 |
| **Code Reviewer** | 交叉 code review、安全审计、确保实现符合设计 |

```
Dev agent ←→ Code Reviewer
    ↑              ↑
    └──  QA agent ──┘
```

## 术语表

| 术语 | 定义 |
|------|------|
| PE | Permission Engine，权限引擎，独立进程运行，不可被篡改 |
| Agent | 具有 LLM 能力的执行单元，可理解指令并通过工具行动 |
| Skill | 封装好的工具能力，可被 agent 调用 |
| IM Adapter | 即时通讯后端适配器，负责协议转换 |
| Rule | 权限规则，声明式定义某 agent 能做什么 |
| seccomp | Linux 内核安全机制，锁定进程可使用的系统调用（syscall），防止提权 |
| landlock | Linux 内核安全机制，细粒度文件系统权限控制，限制进程对指定目录/文件的访问 |
| hot-reload | 运行时动态重新加载配置，无需重启服务 |
| rollback | 配置校验失败时，自动回退到上一可用版本 |
| ConfigProvider | 配置模块的统一抽象 trait，支持版本、校验、回滚 |
| bootstrap agent | 框架启动时默认运行的第一个 agent，即 Root Agent |
| inter-agent 规则 | 控制在什么条件下允许两个 agent 之间直接通信 |
| skill review | 可选的专家审核机制，新增或修改 skill 时需要安全专家评估 |

## OpenClaw 架构参考（低优先级）

> ⚠️ 本节为可选参考，优先级低，应在 Phase 2 以后进行。

OpenClaw 是 CloseClaw 的上游项目（Node.js vs Rust）。如有需要可 clone 并产出架构分析报告，重点记录：
- 哪些设计直接继承
- 哪些是有意改进的
- 踩过的坑
