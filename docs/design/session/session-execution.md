# Session 执行状态

## 概述

Session 执行状态跟踪 session 运行时的所有活跃操作：LLM 交互、工具执行、子 session 执行。三个维度独立跟踪，组合判定 session 当前是否空闲可接收新输入。执行状态为纯内存数据，不进持久化——session resume 后执行状态初始为 Idle。

## 架构

### 三维执行状态

Session 的执行状态由三个独立维度组成，每个维度各自维护自己的状态：

```
ConversationSession
  ├── LLM 状态：Idle | Requesting | Receiving
  │     Idle ──→ LLM 请求发出 → Requesting
  │     Requesting ──→ 首 token 到达 → Receiving（流式）
  │     Requesting ──→ 完整响应返回 → Idle（非流式）
  │     Receiving ──→ 流结束 → Idle
  │
  ├── 工具状态：每个工具调用独立跟踪
  │       Pending → 前台执行 | 后台执行
  │       执行中 → 完成 | 失败 | 被终止 | 超时
  │     前台：session 阻塞，不接受新的 LLM 请求直到完成
  │     后台：session 不阻塞，可继续对话，进程句柄保留
  │
  └── 子 Session 状态：每个子 session 独立跟踪
          执行中 → 完成 | 被终止 | 出错
        子 session 由 spawn 创建，父 session 持有其引用
        子 session 完成时结果通过消息队列注入父 session
```

### 整体状态判定

Session 的整体状态由三维组合判定：

| LLM | 前台 Tool | 后台 Tool | 子 Session | 整体判定 |
|-----|----------|----------|-----------|---------|
| Idle | 无 | 无 | 无 | **Idle**：完全空闲，等待输入 |
| Idle | 无 | 有 | 无 | **Idle（后台活跃）**：可接受新输入，但需提示有后台任务 |
| Idle | 无 | 无 | 有 Running | **Waiting**：等待子 session 完成 |
| Requesting / Receiving | * | * | * | **Busy**：LLM 交互中 |
| * | 有前台 | * | * | **Busy**：工具执行中（阻塞） |

### 级联停止

停止一个 session 时，需清理该 session 拥有的所有活跃资源：

- **子 session**：递归调用每个 child session 的 stop 方法，形成自顶向下的级联停止
- **工具进程**：遍历所有活跃工具调用，对执行中的进程发送 kill 信号。前台和后台都停
- **LLM 请求**：若 LLM 状态为 Requesting 或 Receiving，通过取消机制终止进行中的请求

停止完成后，LLM 状态置 Idle，工具状态和子 Session 状态清空。

级联采用 AbortController 链：父 session 的 AbortController abort 时联动子 session，子 session 单独 abort 不影响父。

### 停止入口

- **斜杠指令**（`/stop`）：用户在 session 内输入，停当前 session。可选 `--cascade` 标记决定是否同时停子 session
- **父 session 停止**：父 session 被停（用户 `/stop --cascade` 或系统级停止）时，级联停止其所有子 session
- **优雅关闭**：系统关闭时，遍历所有活跃 session 逐个执行停止，父 session 先于子 session（避免访问已释放的子 session 句柄）

### 后台结果注入

后台工具完成或子 session 完成时，结果不作为内部事件追加，而是作为消息注入对话流，agent 在下一轮 turn 中直接看到。

注入通过优先级消息队列调度：

- **now**：最高优先级，立即注入（用户输入前）。用于系统级紧急通知
- **next**：下一轮对话中尽早注入。用于卡死警告、子 session 完成通知等需要 agent 及时响应但不超过用户输入的内容
- **later**：在合适时机注入。用于普通后台工具完成通知

通知内容为结构化格式，包含任务标识、完成状态、结果或输出路径。带去重保护——同一任务只注入一次。

## 数据流

### 执行状态转换

```
新 session 创建
  → 所有执行状态初始为空闲
  ↓
收到用户消息
  → 组装 LLM 请求 → LLM 状态变为 Requesting
    → 流式：首 token 到 → Receiving → 流结束 → Idle
    → 非流式：完整响应后 → Idle
  ↓
LLM 返回 tool call
  → 创建工具调用 → 状态为 Pending
    → 前台执行 → 阻塞等待完成 → 完成后注销
    → 后台执行 → 不阻塞 → 进程退出 → 注入结果到消息队列
  ↓
LLM 返回 spawn 请求
  → 创建子 session → 状态为执行中
    → 子 session 执行中，父 session 不阻塞（等待通知）
    → 子 session 完成 → 状态改为完成 → 结果注入父 session 消息队列
  ↓
Session resume（从 archived 恢复）
  → 所有执行状态重置为空闲
  → 对话历史从 transcript 重建
```

### 停止流程

```
触发停止（/stop 或级联或优雅关闭）
  ↓
  确定 cascade 参数
  ↓
  ├── cascade = true：
  │     ↓
  │     遍历所有状态为执行中的子 session
  │       → 对每个子 session 递归调用 stop(cascade=true)
  │     ↓
  │     等待所有子 session 停止完成
  │
  ├── 杀工具进程：
       遍历所有活跃工具调用
         → 前台执行 → 发送 kill 信号
         → 后台执行 → 发送 kill 信号
  │     ↓
  │     等待所有进程终止（带超时）
  │
  ├── 取消 LLM 请求：
  │     若 LLM 状态为 Requesting 或 Receiving
  │       → 触发取消机制
  │     ↓
  │     LLM 状态 = Idle
  │
  └── 清理：
       清空工具状态
       清空子 Session 状态
       ↓
       如果是主动关闭：SessionManager 移除运行时引用
```

### 子 session 完成注入

```
子 session 执行完成
  → 提取子 session 的最后一条 assistant 消息作为结果
  → 父 session 中子 session 状态标记为完成
  → 生成结构化通知消息
  → 入队父 session 消息队列（优先级 next）
  → 带去重保护
  ↓
父 session 下一轮 turn
  → 消息队列出队 → 子 session 完成通知作为消息注入对话流
  → agent 看到通知内容，可据此继续决策
```

## 模块关系

### 上游

- **Gateway**：用户 `/stop` 指令触发 session 停止
- **Daemon**：优雅关闭时遍历所有活跃 session 触发停止
- **父 session**：父 session 停止时级联触发子 session 停止

### 下游

- **LLM Provider**：停止时通过取消机制终止进行中的请求
- **Tools 模块**：kill 工具进程句柄
- **Spawn 协调**：子 session 完成时通过消息队列注入结果

### 无关

- **持久化层**（无调用关系）：执行状态不进 CheckpointManager 持久化，resume 时重置。SessionStatus（Active / Archived）与执行状态独立——archived session 恢复后执行状态为 Idle
- **Permission 模块**（无调用关系）：停止操作不涉及权限检查
- **Processor Chain / Renderer**（无调用关系）：后台结果注入走消息队列，不走出站处理链
