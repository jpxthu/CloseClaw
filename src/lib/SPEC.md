# lib Module Specification

> 本文档按 SPEC_CONVENTION.md v3 标准编写，描述 `src/lib.rs` 模块的精确功能说明。

---

## 模块概述

`src/lib.rs` 是 CloseClaw 库的入口模块，负责导出所有子模块并提供全局初始化函数 `init()`。

模块承担两项职责：
- **模块导出**：通过 `pub mod` 声明式导出所有子模块，构成 CloseClaw 的公共 API 表面
- **初始化**：`init()` 完成 tracing subscriber 的全局初始化，设置日志过滤层级、格式和线程 ID 展示；`init()` 全局只能调用一次

`init()` 是 CloseClaw 进程/库的初始化入口，调用 `tracing_subscriber::fmt().init()` 设置全局 subscriber，由 `env!("CARGO_PKG_VERSION")` 在初始化时广播版本信息。

---

## 公开接口

| 接口 | 说明 |
|------|------|
| `pub mod agent` | Agent 生命周期与注册管理 |
| `pub mod audit` | 审计日志模块 |
| `pub mod card` | 卡片消息渲染模块 |
| `pub mod chat` | 聊天会话模块 |
| `pub mod cli` | 命令行接口模块 |
| `pub mod config` | 配置热加载与验证模块 |
| `pub mod daemon` | 守护进程模块 |
| `pub mod gateway` | 协议网关模块 |
| `pub mod im` | IM 协议适配器模块 |
| `pub mod llm` | LLM 调用封装模块 |
| `pub mod mode` | Agent 模式决策模块 |
| `pub mod permission` | 权限引擎模块 |
| `pub mod platform` | 平台能力抽象模块 |
| `pub mod session` | 会话管理模块 |
| `pub mod skills` | 内置技能模块 |
| `pub mod system_prompt` | System prompt 构建模块 |
| `pub fn init()` | 初始化 tracing subscriber，完成全局日志系统配置 |

---

## 子模块架构

`lib.rs` 下所有子模块均为 `pub mod`，组成 CloseClaw 的顶层模块划分：

```
lib
├── agent        — Agent 生命周期与注册
├── audit        — 审计日志
├── card         — 卡片消息渲染
├── chat         — 聊天会话
├── cli          — 命令行接口
├── config       — 配置热加载与验证
├── daemon       — 守护进程
├── gateway      — IM 协议网关
├── im           — IM 协议适配器
├── llm          — LLM 调用封装
├── mode         — Agent 模式决策
├── permission   — 权限引擎
├── platform     — 平台能力抽象
├── session      — 会话管理
├── skills       — 内置技能
└── system_prompt — System prompt 构建
```

各子模块各自独立，对外通过 `lib::` 命名空间暴露公共 API。
