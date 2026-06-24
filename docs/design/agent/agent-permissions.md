# Agent 权限继承

## 概述

Agent 权限沿 spawn 链路继承，子 agent 的实际权限是目标 agent 自身权限、所有父 agent 当前权限、以及当前用户权限的交集。权限只能收窄，不能放宽。

## 架构

### 权限计算规则

子 agent 权限由三方取交集：

```
子 agent 权限 = 子 AgentConfig.permissions
               ∩ 父 agent 当前权限
               ∩ 当前 User 权限
```

沿 spawn 链路，该规则递归应用：父 agent 的"当前权限"本身也是由其自身配置与更上层父 agent 权限的交集决定。最终形成从根到叶的权限收敛链。

### 约束规则

- 权限只能收窄，不能放宽
- 子 agent 的权限被 Deny 时返回 PermissionDenied 错误给调用方，不进入用户审批流程（子 agent 不是面向用户的入口）

### Workspace 路径授权

子 agent 的 workspace 路径授权规则：

- spawn 参数显式指定 → 使用参数值
- 目标 agent 配置中的 workspace
- 以上均未指定 → 使用父 agent workspace 下的子目录
- workspace 路径的自动授权按子 agent 的 agent_id 和 user_id 重新计算

## 数据流

### 权限评估流程

```
操作请求（Agent A 以 User U 身份执行 Operation O）
  ↓
权限模块评估：
  1. 提取 Agent A 的 permissions 基线规则
  2. 沿 spawn 链路，收集所有父 agent 的当前权限
  3. 计算交集：Agent 链路权限 = 基线 ∩ 父₁ ∩ 父₂ ∩ ... ∩ 根 agent
  4. 同时评估 User U 的权限规则
  5. 最终结果 = Agent 链路权限 ∩ User 权限
      - 双方都 Allow → Allow
      - 任一方 Deny → Deny
      - Owner（User ID = "owner"）→ 跳过 User 维度，仅评估 Agent 维度
  6. 子 agent 被 Deny → 返回 PermissionDenied 错误给调用方
```

### 继承链路示例

```
根 agent（depth=0）
  ├── permissions: { exec: allow, file_write: allow, network: deny }
  │
  └── spawn → 子 agent A（depth=1）
        ├── AgentConfig.permissions: { exec: allow, file_write: deny }
        ├── 父 agent 当前权限:         { exec: allow, file_write: allow, network: deny }
        └── 实际权限:                  { exec: allow, file_write: deny, network: deny }
              （file_write 被 AgentConfig 收窄为 deny）
              │
              └── spawn → 子 agent B（depth=2）
                    ├── AgentConfig.permissions: { exec: allow }
                    ├── 父 agent（A）当前权限:    { exec: allow, file_write: deny, network: deny }
                    └── 实际权限:                 { exec: allow, file_write: deny, network: deny }
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Session 模块（spawn 流程） | spawn 时向 Permission 模块传递 spawn 链路信息，触发权限继承计算 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| — | Agent 权限模块不直接调用其他模块，权限评估由 Permission 模块独立完成 |

### 无关

| 模块 | 说明 |
|------|------|
| Permission | 权限评估由 Permission 模块独立完成，从 Agent 配置文件加载权限规则。Agent 模块不调用 Permission |
| System Prompt | 权限规则不在 bootstrap 中定义 |
| Tools | 工具可见性由 tools/disallowedTools 白名单控制，不做运行时权限判断 |
