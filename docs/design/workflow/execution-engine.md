# Execution Engine

## 概述

Execution Engine 是 workflow 的运行时核心，管理步骤执行的完整生命周期：按定义驱动状态转换、注入协议消息、评估跳转条件。Agent 负责执行步骤内容，Engine 负责流程结构。

## 架构

### 状态机

```
   创建 (workflow_start 或 /workflow)
     │
     ▼
┌──────────┐   进入时注入 goal
│executing │────────────────────────────────┐
└────┬─────┘                                │
     │ Agent turn 结束，session idle         │
     ▼                                      │
┌──────────┐                                │
│verifying │  Engine 注入验收清单             │
└────┬─────┘                                │
     │                                      │
     ├── 未完成 → 继续执行 → idling → 回到 verifying
     ├── 完成 → workflow_verify()            │
     │         │                            │
     │         ▼                            │
     │   ┌──────────┐                       │
     │   │ jumping  │  Engine 注入跳转问题    │
     │   └────┬─────┘                       │
     │        │                             │
     │   ┌────┼────────┐                    │
     │   ▼    ▼        ▼                    │
     │  goto reexecute  done                │
     │   │    │         │                   │
     │   ▼    │         ▼                   │
     │ executing│    complete                │
     │         │                            │
     │         └────────────────────────────┘
     │          (reexecute 回到 executing)
     │
     ├── 超时(死锁) →
     │         ┌──────────┐
     │         │ blocked  │  Engine 通知 owner
     │         └────┬─────┘
     │              │
     │    owner 决策：继续/跳过/终止
     │              │
     │         ┌────┼────┐
     │         ▼    ▼    ▼
     │     继续  跳过  终止
     │         │    │    │
     │         ▼    ▼    ▼
     │    executing next complete
     │
     └── Agent 主动阻塞 →
              │
              ▼
         ┌──────────┐
         │ blocked  │  Agent 调 workflow_blocked({reason})
         └────┬─────┘
              │ owner 回复后 Engine 评估 → verifying
              ▼
          verifying
```

状态说明：

- **executing**：Agent 正在执行步骤内容，Engine 不干预
- **verifying**：Agent turn 结束 session idle 后，Engine 注入验收清单，等待 Agent 响应
- **jumping**：Agent 声明完成（workflow_verify），Engine 注入跳转问题，等待 Agent 回答
- **blocked**：阻塞状态——blocking 步骤等待 owner 输入，或死锁超时
- **complete**：workflow 执行完毕

### 三阶段协议

每个步骤（除 complete）经过三个阶段，由 Engine 驱动转换：

| 阶段 | 触发 | Engine 注入 | Agent 动作 | 消息抹除 |
|------|------|------------|-----------|---------|
| Goal | 进入步骤时 | 步骤目标描述（纯文本） | 执行任务 | 否 |
| Verify | session idle | 验收清单 + "全部满足则调 workflow_verify，否则继续" | 自查 → verify() 或继续 | 是 |
| Jump | Agent 调 verify() | 跳转问题（ABCD 选项） | 回答 → jump({answers}) | 是 |

### 消息角色

所有 Engine 注入的消息使用独立的 `workflow` 角色，与 `user`、`assistant`、`system`、`tool` 并列。Agent 通过消息角色识别流程控制消息，与普通对话区分。

### 空闲判定

Engine 在 session 空闲时触发 verify。空闲条件：

- LLM 不在请求中
- 没有前台阻塞的工具调用
- 所有 Agent 主动发起的子 session 已完成

后台工具执行中不影响空闲判定——Agent 可以在后台任务运行期间继续对话或完成步骤。

## 数据流

### 正常执行流程

```
[Engine] 注入 goal
  role: workflow, type: goal
  内容：当前步骤的目标描述
    ↓
[Agent] 执行步骤（连续工具调用，可 spawn 子 session，可多轮思考）
    ↓ Agent turn 结束，session idle
[Engine] 注入 verify
  role: workflow, type: verify
  内容：
    □ 验收清单条目 1
    □ 验收清单条目 2
    如果全部满足，调用 workflow_verify；否则继续完成步骤。
    ↓
[Agent] 自查
  ├─ 未完成 → 继续执行 → 等下次 idle
  └─ 完成 → workflow_verify()
      ↓
[Engine] 抹除 verify 交互记录（含 tool_call 和 tool_result）
[Engine] 注入 jump
  role: workflow, type: jump
  内容：
    问题 1？（boolean 类型）
    问题 2？（enum 类型）
    A) 选项描述 A
    B) 选项描述 B
    C) 其他
    调用 workflow_jump({ <question1_id>: true|false, <question2_id>: "A"|"B"|"C" })
    ↓
[Agent] workflow_jump({ answers })
    ↓
[Engine] 评估 transitions
  ↓
[Engine] 抹除 jump 交互记录（含 tool_call 和 tool_result）
  ↓
[Engine] 更新 WorkflowRun state（current_step, step_history, step_data）
[Engine] 注入下一步 goal 或结束
```

### 跳转评估

Engine 收到 `workflow_jump({answers})` 后，按 transitions 顺序匹配：

1. 取第一条 transition
2. 检查 `when` 中所有条件是否与 answers 匹配
3. 全部匹配 → 执行该 transition 的 action
4. 有任一不匹配 → 取下一个 transition 继续
5. 所有 when transition 都不匹配 → 执行 `default` transition

匹配是纯硬编码的——布尔比对、枚举匹配、字符串比对。Engine 不做语义理解。

### 跳转动作

**goto(N)**：前进到 Step N。清空 step_data，step_history 追加当前步骤完成记录。

**reexecute(N)**：重入 Step N。保留 step_data，不追加完成记录。goal 注入时附加"重新执行（已保留数据：{step_data 摘要}）"提示。

**complete**：Workflow 结束。Engine 清理 workflow 上下文，session 可继续作为普通 session 使用。

### 验证重试

Agent 未完成验证时可继续执行——Engine 等下次 session idle 自动重新注入 verify。

超时死锁保护作为兜底：

Agent 收到 verify 后长时间无响应：

1. Engine 等待超时（可配置，默认 5 分钟）
2. 超时 → 重新注入 verify（pending_verify.count + 1）
3. 超过重试上限（默认 3 次）→ phase = blocked
4. Engine 通知 owner："Workflow {name} 在 Step {N} 阻塞。"

### 阻塞处理

Blocking 步骤的特殊流程：

```
[Engine] 注入 goal（含"请等待 owner 确认"指令）
[Engine] 将步骤状态标记为 blocked

--- Owner 提供了输入 ---

[Engine] 评估输入 → 标记为 verifying → 正常 verify → jump 流程
```

Agent 主动请求阻塞（等待 owner 回复）：

```
[Agent] workflow_blocked({ reason: "..." })
[Engine] 标记为 blocked → 通知 owner

--- Owner 回复后 ---

[Engine] 评估回复 → 标记为 verifying → 正常 verify → jump 流程
```

超时死锁导致的 blocked 同样走 owner 决策路径——Engine 通知 owner 后等待回复，再进入 verifying。

## 模块关系

### 上游

- **Workflow Definition**：提供 Workflow 结构体，Engine 按 Step 定义驱动执行。
- **Session**：提供空闲判定（三维执行状态），Engine 在 session idle 时触发 verify。
- **Gateway**：blocking 状态通知 owner 时，Engine 通过 Gateway 发送消息。

### 下游

- **Session**：WorkflowRun 状态随 session checkpoint 持久化，由 Engine 写入。
- **Workflow Tools**：Engine 接收 workflow_verify/jump/blocked 工具调用并处理。

### 无关

- **LLM Provider**（无调用关系）：Engine 不直接调用 LLM，通过注入 workflow 消息间接驱动。
