# Agent 权限继承

## 概述

Agent 权限沿 spawn 链路继承，子 agent 的实际权限是子 agent 自身权限、链路中所有父 agent 的实际权限、以及当前用户权限的交集。权限只能收窄，不能放宽。

Owner（User ID 固定为 `"owner"`，系统的最高权限身份）在权限评估中跳过 User 维度，仅受 Agent 维度约束——这是显式特权，凌驾于"只能收窄"原则之上。

## 架构

### 权限计算规则

子 agent 权限由三方取交集：

```
子 agent 实际权限 = 子 agent 配置权限
                  ∩ 链路中所有父 agent 的实际权限
                  ∩ 当前 User 权限
```

交集计算（子 agent ∩ 父 agent ∩ User）、Deny 沿链路传播、权限实时性等机制的语义以 [permission 需求 §F9](../../requirements/permission.md) 为权威源，实现细节由 [permission 设计文档](../permission/README.md)（子 Agent 权限继承）定义。本文档仅在 Agent 视角说明该继承的约束结果：

- 权限只能收窄，不能放宽
- 子 agent 的权限被 Deny 时返回 PermissionDenied 错误给调用方，不进入用户审批流程（子 agent 不是面向用户的入口）
- 交集计算是字段级的：commands/paths 取交集、timeout_ms 取 min、任一维度缺失视为 deny

沿 spawn 链路，该规则递归应用：父 agent 的"实际权限"本身也是由其自身配置与更上层父 agent 权限的交集决定，最终形成从根到叶的权限收敛链。

### Workspace 路径授权

子 agent 的 workspace 路径授权规则（解析顺序与 working-directory.md 一致）：

- spawn 参数显式指定 → 使用参数值
- 子 agent 配置中的 workspace
- 以上均未指定 → 子 agent 默认工作目录 `{config_dir}/workspaces/{child_agent_id}/{user_id}/`
- workspace 路径的自动授权按子 agent 的 agent_id 和 user_id 重新计算

## 数据流

### 权限评估流程

Spawn 时子 agent 的权限评估沿 permission 模块的评估流程执行，Agent 视角的关键路径如下：

```
操作请求（Agent A 以 User U 身份执行 Operation O）
  ↓
每次操作前重新评估（不缓存，实时反映权限变更）
  ↓
权限模块评估（交集计算 + 额外 Deny 检查，详见 permission 模块）：
  最终结果 = Agent 链路权限 ∩ User 权限（Owner 跳过 User 维度）
  - All 维度 Allow → Allow → 放行给调用方，继续执行
  - 任一维度 Deny → Deny
  ↓
子 agent 被 Deny → 返回 PermissionDenied 错误给调用方；
当前操作为子 agent spawn 时不阻塞整个 spawn 流程，仅拒绝当前操作的执行
```

### Workspace 路径

Workspace 路径授权是独立于操作权限的强制机制：每个 Agent-User 组合自动获得其 workspace 路径（`{config_dir}/workspaces/{agent_id}/{user_id}/`）的读写权限。此授权在操作权限评估之前生效——即使 Agent 和 User 的权限规则都未覆盖该路径，workspace 内的文件仍可读写。

### 继承链路示例

```
根 agent（depth=0）
  ├── 配置权限: { exec: allow, file_write: allow, network: deny }
  │
  └── spawn → 子 agent A（depth=1）
        ├── 配置权限:                  { exec: allow, file_write: deny }
        ├── 父 agent 实际权限:          { exec: allow, file_write: allow, network: deny }
        └── A 的实际权限:               { exec: allow, file_write: deny, network: deny }
              （file_write 被子 agent 自身配置收窄为 deny）
              │
              └── spawn → 子 agent B（depth=2）
                    ├── 配置权限:          { exec: allow }
                    ├── 父 agent（A）实际权限: { exec: allow, file_write: deny, network: deny }
                    └── B 的实际权限:        { exec: allow, file_write: deny, network: deny }
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Session 模块（spawn 流程） | spawn 时先执行非权限前置检查（深度、并发数、allowAgents 白名单等，详见 agent-spawn.md），通过后 sessions_spawn 工具经 tools 模块触发 PermissionEngine.evaluate()，执行 Spawn 维度权限校验（交集计算 + 额外 Deny 检查） |

### 下游

| 模块 | 调用关系 |
|------|---------|
| — | Agent 权限模块不直接调用其他模块。权限评估由 Permission Engine 独立完成，Agent 模块仅定义权限规则和继承方式 |

### 共享类型

权限基线规则结构定义见 [agent-config.md](agent-config.md) § 权限配置；权限评估接口由 Permission Engine 提供，Agent 模块仅定义规则和继承语义。

### 无关

| 模块 | 说明 |
|------|------|
| System Prompt | 权限规则不在 bootstrap 中定义 |
| SlashCommand | 斜杠指令权限由 Gateway 层独立处理（三路分流），不涉及 agent 权限继承链路 |
