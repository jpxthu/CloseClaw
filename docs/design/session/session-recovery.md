# Session 重启恢复

## 概述

在 Daemon 重启后自动检测并恢复崩溃前未完成的操作，包括工具调用、子 session 执行和出站消息投递。恢复策略由 LLM 自主决定：系统只告知"发生了什么"，不替 agent 做判断。

## 架构

### PendingOperation 持久化

崩溃恢复的前提是能够知道"崩溃前在执行什么"。每个 session 在发起关键操作前，先将操作记录持久化到 SessionCheckpoint 的 `pending_operations` 字段中（详见 [session-lifecycle.md](session-lifecycle.md) 数据模型节），确认持久化成功后再执行实际操作。操作完成后立即清除。

写入时机和清除时机的通用规则见 [session-lifecycle.md](session-lifecycle.md) 数据模型节。以下给出各操作类型对应的具体触发点：
- **工具调用**：fork 进程前写入
- **子 session spawn**：创建子 session 前写入
- **出站消息**：投递到 IM channel 前写入

清除时机：
- 工具进程返回结果并确认 → 清除
- 子 session 完成并确认 → 清除
- 出站投递收到 ack → 清除

崩溃时，checkpoint 中残留的 pending 项即为"已发起但未确认完成"的操作。存在误报可能（操作已完成但清除 pending 前崩溃），但不会漏报——agent 收到误报后可通过检查工具进程、session 状态自行确认。

### 启动扫描

```
Daemon 启动
  → SessionManager 初始化
    → 加载 status=active 的 session 到映射表（archived 不加载）
    → 扫描每个 active session 的 checkpoint
      → pending_operations 非空 → 标记为 dirty
      → pending_operations 为空 → 正常（无需处理）
    → 防御性扫描：遍历 status=archived 的 session
      → pending_operations 非空 → 先恢复为 active，再注入恢复通知
      → pending_operations 为空 → 跳过
    → 对每个 dirty session 立即注入恢复通知
```

防御性扫描极少命中（Sweeper 归档前检查 pending_operations 为空），但覆盖崩溃发生在归档过程中的极端窗口：status 已改为 archived 但 transcript 尚未完成迁移的状态下，残留的未完成操作仍能被发现和恢复。

恢复通知在启动时立即注入，不等待入站消息。原因：自动化流程（定时任务、webhook 触发）没有 IM 入站消息来激活它们。

### 恢复通知

对每个 dirty session，在 transcript 末尾注入一条 system 消息，列出未完成操作的摘要：

```
[系统] 网关已重启（重启时间: <时间戳>）

以下操作在重启前未完成：
  • 工具调用: exec("kubectl get pods --all-namespaces") — 发起于 <时间>
  • 子 Session: sub_agent_xxx_1234567890_abc — 已运行 <时长>

你可以使用 sessions_list、sessions_history、process 等工具
了解当前状态，自行判断这些操作的结果，并决定后续处理。
```

### 工具调用的恢复表示

对于 pending 中的工具调用，恢复时不仅发送系统通知，还向 transcript 中注入对应的工具失败结果。原因是 LLM 在发起 tool_call 后期望收到 tool_result——直接给失败结果比让它重新理解系统通知更自然。

```
恢复前 transcript 末尾:
  assistant: [tool_call: exec, args: {command: "kubectl get pods"}]

恢复后 transcript 末尾:
  assistant: [tool_call: exec, args: {command: "kubectl get pods"}]
  tool: {"error": "进程中断：网关重启", "tool": "exec", "op_id": "xxx"}
```

LLM 看到的就是"调用了工具，工具失败了"，跟正常运行时工具失败完全一致的处理路径。不需要额外定义恢复专用的交互协议。

### 树状恢复：根优先

session spawn 形成树状结构。恢复时所有 dirty session 都会收到恢复通知，但只有根节点（有入站来源的 session）会主动消费恢复通知并做出决策。

```
父 session A（有入站来源：IM 用户或 webhook）
  ├── 子 session B（spawn 创建，无独立入站来源）
  └── 子 session C（spawn 创建，无独立入站来源）
```

恢复流程：
1. A、B、C 均为 dirty → 全部注入恢复通知
2. B 和 C 没有独立的入站来源——恢复通知注入后不会被主动消费
3. A 恢复时，通过 `sessions_list` 和 `sessions_history` 工具查看 B、C 的状态
4. A 自主决定：重试 B / 重试 C / 读取 B、C 的产出 / 放弃

若一个 session 同时关联了 IM 用户且又是另一个 session 的子节点——它自己也会收到恢复通知并主动恢复。两个入口都能触发恢复，互不冲突。

恢复深度不设硬限制，完全交给 LLM 逐层决策。spawn 深度已有 config 限制（默认 1 层），实际深度有限，每层恢复需一轮 LLM 交互，总耗时可控。

### 出站消息补投

出站消息的投递也作为 PendingOperation 记录。重启后扫描到有未投递的出站消息时，从 transcript 中找到对应消息内容，按记录的投递渠道重新投递。

```
重启扫描 → 发现 dirty session 有 OutboundMessage pending
  → 从 transcript 中找到对应消息
  → 通过记录的投递渠道重新投递
  → 投递成功 → 清除 pending
```

投递渠道来源：记录最后一次成功投递的渠道信息（最后一次入站消息的来源 channel）。Webhook 触发等非 IM 场景按 session 配置的出站渠道。

补投不加去重保护，采用"宁可重复也不遗漏"的策略。若后续发现重复消息过多，再考虑平台级去重机制。

## 数据流

### 崩溃到恢复全路径

```
操作执行中（工具/spawn/出站）
  ↓
Daemon 崩溃
  ↓ (checkpoint 中 pending_operations 残留)
Daemon 重启
  ↓
SessionManager 启动扫描
  → 遍历 status=active 的 session
  → 扫描 checkpoint.pending_operations
    → 非空 → 标记 dirty，构造恢复通知
    → 为空 → 跳过
  → 遍历 status=archived 的 session（防御性）
    → pending_operations 非空 → 恢复为 active → 标记 dirty
  → 对 dirty session：
    → 注入 system 恢复通知（列出未完成操作摘要）
    → 对每个未完成的工具调用：注入 tool_result 失败反馈
  ↓
Session 收到恢复通知
  → LLM 分析通知内容
  → 使用 sessions_list / sessions_history 检查子 session 状态
  → LLM 自主决定恢复策略：重试 / 读取产物 / 放弃
  ↓
恢复完成
  → 清除 pending_operations
  → 持久化 checkpoint
```

### 工具调用恢复全路径

```
崩溃前 transcript 末尾：
  assistant: tool_call(exec, "kubectl get pods")
  checkpoint.pending_operations: [{op_type: ToolCall, tool: exec, args: "kubectl get pods"}]

重启后恢复：
  → 注入 system 通知
  → 注入 tool_result: {"error": "进程中断：网关重启", "tool": "exec", "op_id": "xxx"}
  
LLM 看到的 transcript：
  assistant: tool_call(exec, "kubectl get pods")
  tool: {"error": "进程中断：网关重启", "tool": "exec", "op_id": "xxx"}
  system: [系统] 网关已重启...
  
  → LLM 看到工具失败，自行决定重试或放弃
```

### 子 session 恢复全路径

```
崩溃前：
  父 A: pending_operations: [{op_type: SubSessionSpawn, child: B_id}]
  子 B: pending_operations: [{op_type: ToolCall, ...}]（假设 B 在执行工具）

重启后：
  父 A 恢复 → LLM 收到通知："子 session B 未完成"
  → LLM 调用 sessions_history(B_id) 查看 B 的状态
  → LLM 调用 sessions_list 确认 B 为 dirty（也有未完成操作）
  → LLM 决定：sessions_spawn 重试 B
  → B 重新创建并执行
  → B 完成后结果注入 A
```

## 模块关系

### 上游

- **Daemon**：启动时创建 SessionManager，SessionManager 在其初始化过程中自动执行启动恢复扫描。
- **SessionManager**：持有 key_registry 映射表，协调恢复流程（dirty 检测、通知注入、映射表注册）。
- **CheckpointManager**：提供 SessionCheckpoint 读写能力，恢复依赖其中的 pending_operations 字段。

### 下游

- **ConversationSession**：接收恢复通知和工具失败结果注入 transcript，由 LLM 决定后续处理。
- **SqliteStorage**：启动扫描时通过索引查询所有 active session 的 checkpoint。
- **IM Adapter（出站）**：补投未完成的出站消息。

### 无关

- **Sweeper**：恢复操作仅在 Daemon 启动时执行一次，Sweeper 的定时归档逻辑与此无关。但 Sweeper 归档前会检查 pending_operations 是否为空——非空不归档，因此 dirty session 不会在恢复前被意外归档。
- **Compaction**：恢复流程不触发压缩。恢复通知和工具失败结果的长度远小于正常对话消息，对 token 预算影响可忽略。
- **注入链路**：恢复时不对已持久化的 system prompt 做额外注入；ConversationSession 重建时的注入由生命周期管理负责，恢复模块不参与。
