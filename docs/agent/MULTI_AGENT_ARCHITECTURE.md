# Multi-Agent Hierarchy Architecture

> CloseClaw 多智能体层级架构设计

## 1. 概述

CloseClaw 采用**层级化的多智能体架构**，支持父 Agent → 子 Agent → 孙 Agent 的无限嵌套（受 `max_depth` 配置限制）。每个 Agent 都是独立的工作单元，拥有自己的能力集合和权限边界，通过安全的通讯机制进行协作和经验共享。

```
用户/网关
    │
    ▼
┌─────────────────────────────────────┐
│  根 Agent (Root)                   │
│  - max_depth: 配置最大层级          │
│  - 权限上限: 由网关/用户设定         │
└─────────────────────────────────────┘
    │
    ├──▶ 子 Agent A ──────────────────┼───▶ 子 Agent B （可横向通讯，由父配置）
    │        │                                  │
    │        ▼                                  ▼
    │   ┌─────────┐                       ┌─────────┐
    │   │ 孙 Agent │                       │ 孙 Agent │
    │   │   A1    │                       │   B1    │
    │   └─────────┘                       └─────────┘
    │
    └──▶ 子 Agent C
             │
             ▼
        ┌─────────┐
        │ 孙 Agent │
        │   C1    │
        └─────────┘
```

### 1.1 设计目标

1. **并行开发**：多个项目/模块同时进行，彼此上下文隔离
2. **能力复制**：子 Agent 继承父的能力，快速启动新任务
3. **经验共享**：项目经验沉淀到项目，通用经验上传父节点共享
4. **权限安全**：底层安全设计，权限由规则引擎保障，不可绕过

### 1.2 与 OpenClaw 的关系

- CloseClaw 与 OpenClaw 是**并行独立**的系统
- CloseClaw **兼容 OpenClaw 的 SKILL 和插件**
- 两者共享 SKILL 生态，但各自有独立的运行时

---

## 2. Agent 配置

### 2.1 目录结构

```
~/.closeclaw/
├── agents/
│   ├── root/              # 根 Agent 配置
│   │   ├── config.json    # Agent 自身配置
│   │   └── permissions.json  # 权限配置
│   ├── project-a/         # 子 Agent A
│   │   ├── config.json
│   │   └── permissions.json
│   └── project-b/         # 子 Agent B
│       ├── config.json
│       └── permissions.json
```

### 2.2 Agent 配置 (config.json)

```json
{
  "id": "agent-uuid",
  "name": "项目A开发助手",
  "parent_id": "parent-agent-uuid",
  "max_child_depth": 2,
  "created_at": "2026-03-22T10:00:00Z",
  "state": "running",
  "communication": {
    "outbound": ["parent-agent-uuid"],
    "inbound": ["parent-agent-uuid"]
  }
}
```

### 2.3 权限配置 (permissions.json)

```json
{
  "agent_id": "agent-uuid",
  "permissions": {
    "exec": {
      "allowed": true,
      "limits": {
        "commands": ["/usr/bin/git", "/usr/bin/cargo"],
        "timeout_ms": 300000
      }
    },
    "file_read": {
      "allowed": true,
      "paths": ["/home/admin/code/closeclaw/**"]
    },
    "file_write": {
      "allowed": true,
      "paths": ["/home/admin/code/closeclaw/src/**"]
    },
    "network": {
      "allowed": false
    }
  },
  "inherited_from": "parent-agent-uuid"
}
```

---

## 3. 权限系统

### 3.1 核心原则

| 原则 | 说明 |
|------|------|
| **Agent 不可修改自己** | Agent 不能随意更改自己的权限 |
| **上级授权** | 权限由父 Agent 在自身权限范围内授予子 Agent |
| **底层校验** | 所有权限检查必须经过 CloseClaw 安全模块，无例外 |
| **不可绕过** | 任何操作都无法绕过 Permission Engine |

### 3.2 权限层级

```
网关/用户 (权限上限)
    │
    ▼
根 Agent (继承上限，可授予子)
    │
    ▼
子 Agent (由父授予，不能超过父的权限)
    │
    ▼
孙 Agent (由子授予，不能超过子的权限)
```

### 3.3 权限操作流程

```
Agent 执行操作 (如: exec "rm -rf /")
    │
    ▼
CloseClaw Core 拦截请求
    │
    ▼
Permission Engine 校验
    │
    ├─▶ 允许 → 执行
    │
    └─▶ 拒绝 → 返回错误，记录审计日志
```

### 3.4 权限查询 SKILL

每个 Agent 内置 **permission_skill**（规划中，待实现），允许 Agent 查询自身权限：

```markdown
## permission_skill

### 查询我的权限
使用 `permission_query` 工具查询当前 Agent 的权限配置。

### 权限范围
- `exec`: 命令执行权限
- `file_read`: 文件读取权限
- `file_write`: 文件写入权限
- `network`: 网络访问权限
- `spawn`: 创建子 Agent 权限

### 示例
问: 我可以执行 git 命令吗？
答: 可以。你的 exec 权限允许 /usr/bin/git 命令。
```

---

## 4. 通讯机制

### 4.1 通讯名单

每个 Agent 的配置中有两类名单：

| 名单 | 作用 | 默认值 |
|------|------|--------|
| **outbound** | 允许主动向谁发起通讯 | `["parent_id"]` |
| **inbound** | 允许接收谁的消息 | `["parent_id"]` |

### 4.2 通讯规则

```
1. Agent 建立时，默认只能向上级发起通讯
2. 父 Agent 可以配置子 Agent 之间的横向通讯
3. 所有通讯权限由 CloseClaw 中央仲裁
4. Agent 可以通过 SKILL 查询自己的通讯名单
```

### 4.3 横向通讯配置示例

父 Agent 配置子 Agent A 和 B 之间可以通讯：

```json
{
  "agent_id": "agent-a",
  "communication": {
    "outbound": ["parent-id", "agent-b-id"],
    "inbound": ["parent-id", "agent-b-id"]
  }
}
```

### 4.4 中央仲裁流程

```
Agent A 想发消息给 Agent B
    │
    ▼
检查 A.outbound 是否包含 B
    │
    ├─▶ 否 → 拒绝
    │
    └─▶ 是 → 检查 B.inbound 是否包含 A
              │
              ├─▶ 否 → 拒绝
              │
              └─▶ 是 → 放行，由 CloseClaw 路由
```

### 4.5 通讯方式

| 场景 | 方式 |
|------|------|
| 子 → 父 上报 | Agent 主动发起 |
| 父 → 子 下行推送 | 子执行新动作时拉取父的状态 |
| 子 ↔ 子 横向 | 父配置后，双方均可发起 |

---

## 5. 经验共享机制

### 5.1 经验分类

| 类型 | 定义 | 示例 |
|------|------|------|
| **项目经验** | 只与当前项目相关 | React 组件优化、CloseClaw 某模块设计 |
| **通用经验** | 可跨项目复用 | prompt 编写技巧、代码 review 最佳实践 |

### 5.2 经验流转

```
子 Agent 形成经验
    │
    ▼
验证经验有效性
    │
    ▼
上报父 Agent
    │
    ▼
父 Agent 判定类型
    │
    ├─▶ 通用经验 → 更新自身 → 推送给所有子节点
    │
    ├─▶ 项目经验 → 留在项目文档
    │
    └─▶ 无法判定 → 上报用户决策
```

### 5.3 经验上报触发时机

- 完成功能模块开发时
- 重要代码 review 完成后
- 遇到问题并找到解决方案时
- 定期日报/周报

### 5.4 经验格式

```json
{
  "id": "exp-uuid",
  "agent_id": "agent-uuid",
  "timestamp": "2026-03-22T10:00:00Z",
  "type": "general|project",
  "title": "经验标题",
  "content": "详细描述",
  "context": "适用场景",
  "verified": true
}
```

---

## 6. 层级深度限制

### 6.1 配置方式

每个 Agent 的 `max_child_depth` 配置声明自己能创建的最大子层级：

```json
{
  "name": "项目A",
  "max_child_depth": 2
}
```

### 6.2 校验逻辑

```
Agent 想创建子 Agent
    │
    ▼
计算当前层级深度
    │
    ├─▶ 超过 max_child_depth → 拒绝创建
    │
    └─▶ 未超过 → 允许创建，更新子 Agent 的 max_child_depth
```

### 6.3 设计建议

根据人类社会经验，层级越多信息衰减越严重。建议：

| max_depth | 适用场景 |
|-----------|----------|
| 2-3 | 小型团队/简单项目 |
| 4-5 | 中型团队/复杂项目 |
| > 5 | 需要谨慎评估 |

---

## 7. SKILL 兼容性

### 7.1 OpenClaw SKILL 兼容

CloseClaw 的 SKILL 系统与 OpenClaw 兼容，可以直接使用 OpenClaw 社区的 SKILL。

### 7.2 SKILL 发现机制（规划中，待实现）

```markdown
## skill_discovery_skill

### 查找可用 SKILL
使用 `find_skills` 工具搜索 SKILL 市场。

### 安装 SKILL
使用 `clawhub` 工具安装/更新 SKILL。

### 示例
问: 有没有处理 JSON 的 SKILL？
答: 找到 json_ops SKILL，可以进行 JSON 解析、验证、转换等操作。
```

---

## 8. 尚待明确的 TODO

| 项目 | 状态 | 说明 |
|------|------|------|
| 经验推送机制 | TODO | 父→子的下行推送具体实现方式待定 |
| 通讯延迟处理 | TODO | 消息队列/长连接/拉取策略待定 |
| Agent 生命周期 | TODO | 销毁/暂停/恢复的详细状态机 |

---

## 9. 关键设计决策记录

| 日期 | 决策 | 理由 |
|------|------|------|
| 2026-03-22 | Agent 权限配置在各自目录下 | 自由度更高，由父在权限内修改 |
| 2026-03-22 | 通讯名单由 CloseClaw 中央仲裁 | 安全第一，所有权限相关都要过安全模块 |
| 2026-03-22 | 经验类型由父最终判定 | 父有全局视野，判定更准确 |
| 2026-03-22 | max_depth 由 CloseClaw 逻辑校验 | 确保层级限制不可绕过 |
