# CLI Admin

## 概述

CLI Admin 是 CLI 的管理命令层，提供对 daemon 的直接管理操作。所有命令绕过消息链路——不经 Processor Chain、不经 Session、不经 Gateway 路由。

## 架构

Admin 命令通过 `closeclaw <command> [args]` 调用，由 handler 函数执行。分为本地操作和 daemon RPC 两类。

```
closeclaw <command> [args]
  ↓
参数解析（clap）
  ↓
handler 函数
  ├── 本地操作（不需要 daemon 运行）
  │     ├── run         — 启动 daemon 进程
  │     ├── stop        — 终止 daemon 进程
  │     ├── config      — 读/写配置文件
  │     └── rule        — 读/写权限规则
  │
  └── daemon RPC（需要 daemon 已运行）
        ├── agent       — 查询/管理 agent
        └── skill       — 查询/安装 skill
```

### 与斜杠指令的关系

部分 Slash 指令和 Admin 命令功能重叠（如 status 查询）。区别在于：
- Admin 命令可操作的范围更广（启停 daemon、交互式配置向导）
- Admin 命令仅 Owner 可执行（CLI 渠道调用者默认为 Owner，User ID 固定为 "owner"，不走 Permission 引擎）
- Slash 指令需要 session 上下文，Admin 命令不需要

### 命令分类

**本地操作**：不依赖 daemon 运行状态，直接操作文件系统或进程。

run 命令启动 daemon 进程，读取配置目录、初始化所有组件后进入消息循环。stop 命令读取 PID 文件，向 daemon 进程发送终止信号，清理 PID 文件。

config 命令管理配置文件。setup 子命令启动交互式配置向导（详见 [LLM Provider 配置向导](../llm/provider-config-wizard.md)），引导用户选择 Provider、输入凭据、发现模型并写入配置。validate 子命令校验配置文件格式。list 子命令列出 `~/.closeclaw/` 目录下的所有配置文件（models.json、channels.json、gateway.json、plugins.json、system.json、credentials.json），显示文件路径、版本号和最后修改时间。

rule 命令管理权限规则。check 子命令校验单条规则语法，list 子命令列出已有规则。

**daemon RPC**：依赖 daemon 已运行，通过管理协议查询或操作 daemon 状态。

agent 命令管理 agent 实例（创建、查询、列表）。skill 命令管理已安装的 skill（列表、安装）。

## 数据流

```
closeclaw <command> [args]
  ↓
参数解析 → 确定命令类型
  ↓
┌─ 本地操作
│   ├── run：启动 daemon 子进程 → 等待运行中
│   ├── stop：读 PID 文件 → kill 进程 → 清理 PID 文件
│   ├── config setup：交互式向导 → 拉取模型列表 → 用户选择 → 写入配置
│   ├── config validate：读文件 → 校验格式 → 输出结果
│   ├── config list：扫描配置目录 → 列出配置文件 → 输出列表
│   └── rule check/list：读规则文件 → 校验/列表 → 输出结果
│
└─ daemon RPC
    ├── agent：发送 RPC → daemon 查询/操作 → 返回结果
    └── skill：发送 RPC → daemon 查询/操作 → 返回结果
  ↓
stdout：格式化文本 / 表格 / JSON
```

## 模块关系

- **上游**：操作系统命令行参数
- **下游**：Daemon（run 创建 daemon 实例，stop 终止 daemon 进程，agent/skill RPC 查询 daemon 状态）、Config 模块（config setup 写 models.json 和凭据文件；config validate 校验配置格式）、Permission 模块（rule 命令管理权限规则）、LLM 模块（config setup 向导中调用模型发现能力）
- **与模块内其他子功能**：与 CLI Chat 共享 platform 层（PID 文件、配置目录、进程信号），但不共享消息链路
- **无关**：Gateway（Admin 命令不经 Gateway 路由）、Processor Chain（Admin 命令不经消息处理链）、Session（Admin 命令无 session 上下文）
