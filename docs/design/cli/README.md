# CLI

## 概述

- 关联需求文档：[requirements/cli.md](../../requirements/cli.md)
- 核心职责：CLI 是 CloseClaw 的命令行接口模块。它包含两个子系统：通过终端进行对话交互的 CLI Chat（terminal 消息渠道的 IMPlugin 实现）和对 daemon 的管理操作（CLI Admin）。

## 架构

CLI 模块分为两个子系统。CLI Chat 以 platform="terminal" 注册到 Gateway 的 Plugin Registry，走完整出入站链路（实现实体 TerminalPlugin 包含 TerminalAdapter（入站解析）与 TerminalRenderer（出站渲染）两个组件）；Chat 依赖 daemon 已运行——启动时连接 daemon 管理接口，不可达则报错退出并提示先执行 `closeclaw run`，不自动拉起。CLI Admin 绕过消息链路，命令分为本地操作（不经 daemon）与 daemon RPC（经 daemon 管理接口 / Admin RPC Server）两类。

```
closeclaw <command>

├── Chat 层（消息链路内）
│   └── closeclaw chat
│       └── TerminalPlugin（实现 IM 插件接口，以 terminal 渠道注册）
│           ├── 入站：stdin → TerminalAdapter → NormalizedMessage →（经 Processor Chain 入站处理后交由 Gateway 路由）
│           └── 出站：（Gateway Processor Chain 出站产出 ContentBlock[]）→ TerminalRenderer → RenderedOutput → TerminalPlugin 发送 → stdout
│
└── Admin 层（消息链路外）
    ├── closeclaw run          — 启动 daemon
    ├── closeclaw stop         — 停止 daemon
    ├── closeclaw config       — 管理配置文件
    ├── closeclaw agent        — 查询 agent（列表、详情）
    ├── closeclaw rule         — 校验规则语法 / 列出已有规则（只读）
    └── closeclaw skill        — 查询 skill（列表）
```

### Chat 层与 IM 渠道的关系

CLI Chat 与飞书、Discord 等 IM 渠道实现同一个 IMPlugin trait（接口契约见 [common/core-traits](../common/core-traits.md#implugin)），在 Gateway 的 Plugin Registry 中平级注册。差异全部封装在 TerminalPlugin 内部：入站走 stdin、出站走 stdout、调用者默认为 Owner（单用户）无需鉴权。terminal 渠道的实现细节见 [CLI Chat](chat.md)。

### 跨操作系统

CLI 支持 Linux、macOS 及 Windows（经 WSL2，行为等同 Linux）。OS 差异通过 [platform 模块](../platform/README.md) 做薄层封装，CLI 的业务逻辑不感知操作系统差异。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [CLI Chat](chat.md) | TerminalPlugin：terminal 渠道的 IM 插件实现，入站解析 stdin 到 NormalizedMessage，出站渲染后发送到 stdout |
| [Terminal Renderer](renderer.md) | ContentBlock[] 到终端文本（ANSI / 纯文本，按终端能力）的渲染策略；流式渲染经 common 流式渲染原语，不走本组件 |
| [CLI Admin](admin.md) | 管理命令体系：daemon 生命周期、配置管理、资源查询 |

## 数据流

### Chat 层

0. 启动检查：连接 daemon 管理接口，不可达则报错退出并提示先执行 `closeclaw run`（不自动拉起）
1. stdin 输入，TerminalAdapter 解析为 NormalizedMessage（terminal 渠道专用字段值见 chat.md）
2. Processor Chain 入站处理后，消息进入 Gateway 路由，按内容分流：
   - 审批指令（`/approve-once`、`/approve-whitelist`、`/deny`）→ Gateway 硬拦截走权限审批流，不进 SlashDispatcher
   - 其他以 `/` 开头 → SlashDispatcher → SlashHandler/SlashResult → ContentBlock[]
   - 普通文本 → Session → LLM → ContentBlock[]
3. ContentBlock[] 经 Processor Chain 出站（VerbosityFilter → DslParser → OutboundRawLog）到达 TerminalPlugin
4. TerminalPlugin 先渲染后发送：TerminalRenderer 渲染 → RenderedOutput（按终端能力为 ANSI 或纯文本），随后 Gateway 在渲染与发送之间执行出站中间件链（审计、频率限制等，通过后才发送），再由 TerminalPlugin 发送 → stdout

> **流式路径**：LLM 流式输出时不走上述批量路径——增量阶段开始前 Gateway 先执行一次 pre-flight 出站中间件检查（被拒则终止流式并发送拒绝通知）；随后 [StreamEvent](../common/shared-types.md#streamevent) 事件流经统一预处理（VerbosityFilter 按块边界过滤 → DslParser 透传）后，TerminalPlugin 委托 common 流式渲染原语（[StreamingRenderer](../common/core-traits.md#streamingrenderer)）逐事件产出 [StreamingOutput](../common/shared-types.md#streamingoutput) 增量输出，逐片写入 stdout；流式结束后由 Gateway 在收尾阶段调度 DslParser 完整解析与 OutboundRawLog 出站日志。详见 [CLI Chat](chat.md) 与 [IM Adapter 流式渲染](../im_adapter/streaming-render.md)。

### Admin 层

1. `closeclaw <command> [args]` 输入，参数解析后由对应 handler 函数执行
2. 按命令类型分派执行：
   - 本地操作（run 启动 daemon、stop 终止 daemon 进程、config 读/写配置、rule 校验/列出规则，不经 daemon）
   - daemon RPC（agent/skill 等远程查询调用，经管理接口发往 daemon；依赖 daemon 已运行，不可达时报错退出）
3. 两类命令的结果均落到 stdout（状态提示）、文件写入或进程管理副作用

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

- **上游**：操作系统终端（stdin / 命令参数）、Gateway（Chat 层出站方向通过 IMPlugin trait 调用 TerminalPlugin 发送渲染结果）、daemon（Chat 启动时经管理接口做存活检测；Admin 层经管理接口发起 daemon RPC 并消费其返回的查询结果——daemon README 登记 Admin RPC Server）、platform 模块（提供终端能力检测结果与进程/PID 状态等 OS 抽象）
- **下游**：Gateway（Chat 层产 NormalizedMessage 入站，经 Processor Chain 产出 ProcessedMessage 供 Gateway 路由；消费 ContentBlock[] 出站）、daemon（run/stop 启停；agent/skill 命令经管理接口查询 daemon 状态）、Config 模块（config 命令经 config 模块读写配置）、Permission 模块（rule 命令只读查看权限规则）、LLM 模块（config setup 向导调用模型发现）、Agent 模块（config setup 首次配置时创建初始 Agent）
- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：IMPlugin；消费：StreamingRenderer）、[common/shared-types](../common/shared-types.md)（消费：NormalizedMessage、ProcessedMessage、ContentBlock、DslParseResult、RenderedOutput、StreamEvent、StreamingOutput、ContentSegment）
- **无关**：IM Adapter 各平台实现（飞书/Discord 等 platforms/ 实现与本模块的 terminal 插件无相互调用；terminal 作为非 platforms/ 插件经显式注册加入 Plugin Registry）
