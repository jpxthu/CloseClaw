# Agent Model — Agent 定义、层级与通信

## 什么是"一只 agent"

- 独立的 Rust 进程（可配置）
- 持有一个 LLM 会话（可接入多个 LLM 提供者）
- 拥有自己的人设（system prompt）、skill 集合、权限配置
- 通过 Permission Engine 与系统资源交互
- 通过 Agent Registry 与其他 agent 通信

## 层级与继承

```
Root Agent (Bootstrap Agent)
  框架启动时默认运行的第一个 agent
      │
      ├── dev-01  (继承 root + 额外限制)
      ├── dev-02  (继承 root + 额外限制)
      └── qa-01   (继承 root + 额外限制)
```

- **继承**：child agent 默认继承 parent 的所有权限
- **覆盖**：child 可在父权限基础上添加额外限制（收紧，不可放松）
- **平级**：同层级 agent 默认无法互相通信

## Agent 配置示例（agents.json）

```json
{
  "agents": [
    {
      "id": "dev-01",
      "name": "开发助手",
      "parent": "root",
      "persona": "你是 CloseClaw 的开发助手...",
      "skills": ["file_ops", "git_ops"],
      "llm": { "provider": "openai", "model": "gpt-4o" }
    }
  ]
}
```

## Inter-Agent 通信

- agent 之间不共享内存，通过结构化消息通信
- 消息经过 Permission Engine 的 `inter_agent` 规则审查
- 支持：request/response、event/notification、bidirectional stream

**例外情况（显式配置）：**
1. `inter_agent` 规则显式授权
2. 通过 Agent Registry 转发
3. 共享消息队列

## 风险

| 问题 | 状态 |
|------|------|
| agent 通信协议 | 草案（Phase 8） |
| wire format | 待定义（Phase 8） |
