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

沿 spawn 链路，该规则递归应用：父 agent 的"实际权限"本身也是由其自身配置与更上层父 agent 权限的交集决定。最终形成从根到叶的权限收敛链。

### 约束规则

- 权限只能收窄，不能放宽
- 子 agent 的权限被 Deny 时返回 PermissionDenied 错误给调用方，不进入用户审批流程（子 agent 不是面向用户的入口）

### Deny 沿链路传播

链路中所有父 agent 的 Deny 规则作为额外约束，传入子 agent 的权限评估。即使子 agent 自身配置允许某项操作，父 agent 的 Deny 也覆写为禁止。

传播方式：

- 链路中所有父 agent 的 Deny 规则中，以 Agent 维度（AgentOnly 规则，即仅匹配 agent ID、不限制 user ID 的规则）生效的规则被提取
- 提取时将各规则的 agent ID 替换为子 agent ID
- 子 agent 的权限评估在正常规则匹配完成后，额外做一次 Deny 检查：如果本次操作的发起者（caller）匹配任一父 agent 的 Deny 主体约束（subject），立即 Deny

上述"额外 Deny 检查"与权限计算交集（子 ∩ 父 ∩ User）是互补关系，不冗余：

- **交集**解决的是维度级收窄（父 agent 对"文件写入"整体 Deny → 子 agent 的 file_write 维度自然 Deny）
- **额外 Deny 检查**解决的是主体级收窄（父 agent 的 Deny 规则仅匹配特定主体时，该约束需显式传递给子 agent）

沿 spawn 链每增加一级深度，Deny 约束集只增不减。

### 权限实时性

权限评估每次新鲜计算，不缓存评估结果。在同一会话中，权限可能因 Owner 审批通过等原因发生变更，下一次评估立即反映最新状态。

### Workspace 路径授权

子 agent 的 workspace 路径授权规则：

- spawn 参数显式指定 → 使用参数值
- 子 agent 配置中的 workspace
- 以上均未指定 → 使用父 agent workspace 下的子目录
- workspace 路径的自动授权按子 agent 的 agent_id 和 user_id 重新计算

## 数据流

### 权限评估流程

```
操作请求（Agent A 以 User U 身份执行 Operation O）
  ↓
每次操作前重新评估（不缓存，实时反映权限变更）
  ↓
权限模块评估：
  1. 提取 Agent A 的配置权限基线
  2. 沿 spawn 链路，收集链路中所有父 agent 的实际权限
  3. 计算交集：Agent 链路权限 = A 的配置权限 ∩ 链路中所有父 agent 的实际权限
  4. 同时评估 User U 的权限规则
  5. 最终结果 = Agent 链路权限 ∩ User 权限
      - 双方都 Allow → Allow → 放行给调用方，继续执行
      - 任一方 Deny → Deny → 进入步骤 6
      - Owner（User ID = "owner"）→ 跳过 User 维度，仅评估 Agent 维度
  6. 额外 Deny 检查：链路中父 agent 传入的 Deny 主体约束中匹配到 caller → 立即 Deny
  7. 子 agent 被 Deny → 返回 PermissionDenied 错误给调用方；
     当前操作为子 agent spawn 时不阻塞整个 spawn 流程，仅拒绝当前操作的执行
```

### Workspace 路径

Workspace 路径授权是独立于操作权限的强制机制：每个 Agent-User 组合自动获得其 workspace 路径（`{数据目录}/workspaces/{agent_id}/{user_id}/`）的读写权限。此授权在操作权限评估之前生效——即使 Agent 和 User 的权限规则都未覆盖该路径，workspace 内的文件仍可读写。

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

### 无关

| 模块 | 说明 |
|------|------|
| System Prompt | 权限规则不在 bootstrap 中定义 |
| SlashCommand | 斜杠指令权限由 Gateway 层独立处理（三路分流），不涉及 agent 权限继承链路 |
