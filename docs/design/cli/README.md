# CLI

## 概述

- 关联需求文档：[requirements/cli.md](../requirements/cli.md)
- 核心职责：CLI 是 CloseClaw 的命令行接口模块。它包含两层：通过终端进行对话交互的 CLI Chat（terminal 消息渠道的 IMPlugin 实现）和对 daemon 的直接管理操作（CLI Admin）。

## 架构

CLI 模块分为两个子系统。CLI Chat 以 platform="terminal" 注册到 Gateway 的 Plugin Registry，走完整出入站链路。CLI Admin 绕过消息链路，通过独立管理协议与 daemon 交互。

```
closeclaw <command>

├── Chat 层（消息链路内）
│   └── closeclaw chat
│       └── TerminalPlugin（实现 IM 插件接口，以 terminal 渠道注册）
│           ├── 入站：stdin → TerminalAdapter → NormalizedMessage →（交由 Gateway Processor Chain 入站处理后路由）
│           └── 出站：（Gateway Processor Chain 出站产出 ContentBlock[]）→ TerminalRenderer → RenderedOutput → TerminalPlugin 发送 → stdout
│
└── Admin 层（消息链路外）
    ├── closeclaw run          — 启动 daemon
    ├── closeclaw stop         — 停止 daemon
    ├── closeclaw config       — 管理配置文件
    ├── closeclaw agent        — 管理 agent
    ├── closeclaw rule         — 查看权限规则（只读）
    └── closeclaw skill        — 管理 skill
```

### Chat 层与 IM 渠道的关系

CLI Chat 与飞书、Discord 等 IM 渠道实现同一个 IMPlugin trait（接口契约见 [common/core-traits](../common/core-traits.md#implugin)），在 Gateway 的 Plugin Registry 中平级注册。差异全部封装在 TerminalPlugin 内部：入站走 stdin、出站走 stdout、调用者默认为 Owner（单用户）无需鉴权。terminal 渠道的实现细节见 [CLI Chat](chat.md)。

### 跨操作系统

CLI 支持 Linux、macOS 及 Windows（经 WSL2，行为等同 Linux）。OS 差异通过 [platform 模块](../platform/README.md) 做薄层封装，CLI 的业务逻辑不感知操作系统差异。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [CLI Chat](chat.md) | TerminalPlugin：terminal 渠道的 IM 插件实现，入站解析 stdin 到 NormalizedMessage，出站渲染后发送到 stdout |
| [Terminal Renderer](renderer.md) | ContentBlock[] 到 ANSI 终端文本的渲染策略 |
| [CLI Admin](admin.md) | 管理命令体系：daemon 生命周期、配置管理、资源查询 |

## 数据流

### Chat 层

1. stdin 输入，TerminalAdapter 解析为 NormalizedMessage（terminal 渠道专用字段值见 chat.md）
2. Processor Chain 入站处理后，消息进入 Gateway 路由，按内容分流：
   - 以 `/` 开头 → SlashDispatcher → ContentBlock[]
   - 普通文本 → Session → LLM → ContentBlock[]
3. ContentBlock[] 经 Processor Chain 出站（VerbosityFilter → DslParser → OutboundRawLog）到达 TerminalPlugin
4. TerminalPlugin 先渲染后发送，两步顺序执行：TerminalRenderer 渲染 → RenderedOutput（ANSI 文本），随后 TerminalPlugin 发送 → stdout

### Admin 层

1. `closeclaw <command> [args]` 输入，参数解析后由对应 handler 函数执行
2. 按命令类型分派执行：
   - 本地操作（stop 终止 daemon 进程、config setup 写入文件等，不经 daemon）
   - daemon RPC（agent/skill 等远程管理调用，经管理协议发往 daemon）
3. 两类命令的结果均落到 stdout（状态提示）、文件写入或进程管理副作用

## 模块关系

- **上游**：操作系统终端（stdin / 命令参数）、用户、Gateway（Chat 层出站方向通过 IMPlugin trait 调用 TerminalPlugin 发送渲染结果）
- **下游**：Gateway（Chat 层产 NormalizedMessage 入站路由，消费 ContentBlock[] 出站）、daemon（run/stop 启停；agent/skill 命令经管理 RPC 查询与操作 daemon 状态）、Config 模块（config 命令写配置）、Permission 模块（rule 命令只读查看权限规则）、LLM 模块（config setup 向导调用模型发现）
- **无关**：IM Adapter 各平台实现（terminal 渠道与飞书/Discord 平级，实现位于 cli/ 模块，无相互调用）
