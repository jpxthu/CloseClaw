# CLI

## 概述

CLI 是 CloseClaw 的命令行接口模块。它包含两层：通过终端进行对话交互的 CLI Chat（terminal 消息渠道的 IMPlugin 实现）和对 daemon 的直接管理操作（CLI Admin）。

## 架构

CLI 模块分为两个子系统。CLI Chat 以 platform="terminal" 注册到 Gateway 的 Plugin Registry，走完整出入站链路。CLI Admin 绕过消息链路，通过独立管理协议与 daemon 交互。

```
closeclaw <command>

├── Chat 层（消息链路内）
│   └── closeclaw chat
│       └── TerminalPlugin（实现 IMPlugin trait，platform="terminal"）
│           ├── 入站：stdin → TerminalAdapter → NormalizedMessage
│           └── 出站：ContentBlock[] → TerminalRenderer → stdout
│
└── Admin 层（消息链路外）
    ├── closeclaw run          — 启动 daemon
    ├── closeclaw stop         — 停止 daemon
    ├── closeclaw config       — 管理配置文件
    ├── closeclaw agent        — 管理 agent
    ├── closeclaw rule         — 管理权限规则
    └── closeclaw skill        — 管理 skill
```

### Chat 层与 IM 渠道的关系

CLI Chat 与飞书、Discord 等 IM 渠道实现同一个 IMPlugin trait，在 Gateway 的 Plugin Registry 中平级注册。不同之处封装在 TerminalPlugin 内部：
- 入站渠道：stdin（无 webhook）
- 出站渠道：stdout（无 IM API）
- 权限：调用者默认为 Owner（单用户），无需鉴权

斜杠指令和普通对话的语义与其他渠道完全一致——同一套 SlashDispatcher 处理，Gateway 按统一规则路由。

### 跨操作系统

CLI 支持 Linux、macOS、Windows。OS 差异通过 [platform 模块](../platform/README.md) 做薄层封装，CLI 的业务逻辑不感知操作系统差异。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [CLI Chat](chat.md) | TerminalPlugin：从 stdin 解析 NormalizedMessage，经出站链后渲染到 stdout |
| [Terminal Renderer](renderer.md) | ContentBlock[] 到 ANSI 终端文本的渲染策略 |
| [CLI Admin](admin.md) | 管理命令体系：daemon 生命周期、配置管理、资源查询 |

## 数据流

### Chat 层

```
stdin 输入
  ↓
TerminalAdapter.parse() → NormalizedMessage { platform: "terminal", sender_id, peer_id, content, timestamp }
  ↓
Processor Chain 入站 → ProcessedMessage → Gateway 路由
  ├── / 开头 → SlashDispatcher
  └── 普通文本 → Session → LLM → ContentBlock[]
  ↓
Processor Chain 出站 → TerminalRenderer → ANSI 文本
  ↓
stdout
```

### Admin 层

```
closeclaw <command> [args]
  ↓
参数解析 → handler 函数
  ├── 本地操作（stop：终止 daemon 进程；config setup：写入文件）
  └── daemon RPC（远程管理调用）
  ↓
stdout / 文件写入 / 进程管理
```

## 模块关系

- **上游**：操作系统终端（stdin / 命令参数）、用户
- **下游**：Gateway（Chat 层产 NormalizedMessage 入站、消费 ContentBlock[] 出站）、Daemon（run/stop 启停、管理 RPC）、Config 模块（config 命令写配置）、Permission 模块（rule 命令管理权限规则）、LLM 模块（config setup 向导调用模型发现）
- **无关**：IM Adapter 各平台实现（terminal 渠道与飞书/Discord 平级，实现位于 cli/ 模块，无相互调用）
