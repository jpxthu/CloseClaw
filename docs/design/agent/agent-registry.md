# AgentRegistry

## 概述

AgentRegistry 是 Agent 模块的运行时配置查询入口，以 `agent_id` 为键提供 `ResolvedAgentConfig` 的只读查找。启动时由 Daemon 从 Config 加载结果一次性填充，运行时只读查询。

## 架构

AgentRegistry 是轻量级内存查找表，不管理 agent 生命周期、不做权限校验、不做配置合并——这些职责分别属于 Session、Permission、Config 模块。

```
ConfigManager.load_agents()
      │
      │  Vec<ResolvedAgentConfig>
      ▼
AgentRegistry
      │
      │  get(agent_id) → Option<&ResolvedAgentConfig>
      ▼
Session / Permission / System Prompt / ...
```

接口职责：

- 启动时由 Daemon 从 ConfigManager 加载结果一次性填充注册表
- 运行时各消费模块通过 agent_id 查询获取只读配置

**热重载策略**：Config Hot Reload 检测到 agent 配置变更 → 重新加载 → 通知 Daemon → Daemon 触发全量替换。已运行的 session 是否感知变更由各消费模块自行决定——AgentRegistry 只负责提供最新数据，不推送变更通知。

## 数据流

### 启动填充

```
Daemon 启动
  ↓
ConfigManager.load_agents() → Vec<ResolvedAgentConfig>
  ↓
AgentRegistry 填充
  ↓
注册表就绪，各消费模块可查询
```

### 运行时查询

```
模块需要 agent 配置
  ↓
AgentRegistry 查询
  ├── 命中 → 返回 ResolvedAgentConfig
  └── 未命中 → None（调用方自行处理，通常是配置缺失错误）
```

### 热重载

```
Config Hot Reload 检测 agent 配置变更
  ↓
ConfigManager.load_agents() → 新的 Vec<ResolvedAgentConfig>
  ↓
Daemon 触发 AgentRegistry 全量替换
  ↓
注册表内容替换，消费模块下次查询获取新数据
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Daemon | 启动时调用 `populate()` 填充注册表 |
| Config Hot Reload | 检测到 agent 配置变更后通知 Daemon，Daemon 调用 reload() 更新注册表 |
| Config | 提供 `ResolvedAgentConfig` 数据源（Config 只负责文件 IO 和合并，不参与注册） |

### 下游

| 模块 | 消费方式 |
|------|---------|
| Session | 创建 session 时查询 agent 配置（模型、workspace、工具集、skill 列表等） |
| System Prompt | 查询 bootstrap 模式配置 |
| Skills Registry | 查询 agent 的 skills 白名单 |
| Tools Registry | 查询 agent 的 tools 白名单 / 黑名单 |

### 无关

| 模块 | 说明 |
|------|------|
| Gateway | AgentRegistry 不参与消息路由 |
| IM Adapter | AgentRegistry 不涉及平台通信 |
| Processor Chain | AgentRegistry 不参与消息处理 |
