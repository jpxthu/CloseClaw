# LLM 调用失败 Fallback 策略设计方案

## 背景与问题

当前 CloseClaw 的 LLM 调用（通过 OpenClaw）在遇到失败时缺乏系统性的重试和降级机制，可能导致：
- 偶发网络抖动 → 直接失败，用户体验差
- Rate Limit / 配额耗尽 → 没有等待重试，直接死掉
- API Key 失效 → 没有轮换机制
- 主模型不可用 → 无法自动切换到备用模型

## OpenClaw 参考方案分析

OpenClaw 实现了**两阶段 Failover 机制**：

```
Stage 1: Auth Profile Rotation（同一 Provider 内的凭证轮换）
    ↓  all profiles exhausted
Stage 2: Model Fallback（切换到 fallback chain 中的下一个模型）
    ↓  all models exhausted
Stage 3: 对用户报错
```

### 核心机制

| 机制 | 说明 |
|------|------|
| **Cooldown** | 失败后进入冷却期，指数退避（1min → 5min → 25min → 1h cap） |
| **Billing Disable** | 配额不足类错误标记为 disabled，更长退避（5h → 10h → 24h cap） |
| **Session Stickiness** | 认证 Profile 按 Session 绑定，避免每次请求都换凭证导致缓存失效 |
| **User Override** | 用户可以锁定特定模型/Profile，不受自动轮换影响 |

### OpenClaw 可配置项（参考）

```json5
{
  agents: {
    defaults: {
      model: {
        primary: "anthropic/claude-sonnet-4-5",
        fallbacks: ["openai/gpt-4.2", "openrouter/moonshotai/kimi-k2"],
      },
    },
  },
  auth: {
    profiles: { /* ... */ },
    order: { "openai": ["openai:user1@gmail.com", "openai:default"] },
    cooldowns: {
      failureWindowHours: 24,
      billingBackoffHours: 5,
      billingMaxHours: 24,
    },
  },
}
```

## CloseClaw 设计方案

### 1. 错误分类

LLM 调用失败首先需要分类，以决定后续动作：

| 错误类型 | HTTP Code | 典型场景 | 处理策略 |
|----------|-----------|----------|----------|
| **Transient** | 429, 500, 502, 503, 504, timeout | 网络抖动、服务端临时不可用 | **重试**（指数退避） |
| **Auth** | 401, 403 | API Key 过期/无效、权限不足 | **轮换凭证**，不重试当前凭证 |
| **Billing** | 402, 429 (quota exhausted) | 配额耗尽、余额不足 | **禁用凭证**（长退避） |
| **Invalid Request** | 400, 422 | 参数错误、Prompt 超长 | **不重试**，直接切换模型 |
| **Unknown** | 其他 | 未知错误 | **记录+告警**，按 Transient 处理（最多 1 次重试） |

### 2. 两阶段 Failover 流程

```
LLM 调用请求
    │
    ▼
┌─────────────────────┐
│ Stage 1: 尝试当前模型 │
│  - 使用当前 session  │
│    绑定的 auth profile│
└──────────┬──────────┘
           │ 失败
           ▼
    错误分类判断
           │
     ┌─────┴─────┐
     │            │
 Transient?   Auth?
     │            │
  重试(退避)   轮换 auth profile
  3次为上限    重试1次
     │            │
     ▼            ▼
   再失败      再失败
     │            │
     ▼            ▼
Stage 2:        Stage 2:
Model Fallback  Model Fallback
           │
           ▼
    检查 fallback chain
    - 还有下一个模型? → 用新模型 + 第一个 auth profile 重试 Stage 1
    - 没有更多模型了? → 向用户报错
```

### 3. Cooldown 机制

每个 (provider, model) 组合维护独立的 cooldown 状态：

```go
type ModelCooldown struct {
    attempts      int       // 连续失败次数
    cooldownUntil time.Time // 冷却期截止时间
    reason        string    // "transient" | "billing" | "auth"
}
```

**退避策略：**
- Transient: `min(1h, 1min * 2^attempts)`，最大 5 次退避后放弃
- Billing: `min(24h, 5h * 2^attempts)`
- 成功调用后重置计数器

### 4. Session Auth Binding

为避免每次调用都换凭证（导致 Provider 端缓存失效、rate limit 计数不稳定）：

- 每个 Session 绑定当前使用的 (model, auth_profile)
- 仅在以下情况切换：
  - 当前 profile 进入 cooldown / disabled
  - Session 重置（`/new`）
  - 用户手动切换模型

### 5. Fallback Chain 配置

```yaml
llm:
  primary: "minimax/MiniMax-M2.7"
  fallbacks:
    - "dashscope/qwen3-max-2025-09-23"
    - "dashscope/qwen3-vl-plus"
  retry:
    maxAttempts: 3
    minDelayMs: 1000
    maxDelayMs: 30000
    jitter: 0.1
```

### 6. 用户可见行为

| 场景 | 用户看到 |
|------|----------|
| 正在重试（等待中） | 机器人无响应 / thinking...（最多等 30s total） |
| 切换到 fallback 模型 | 无感知（静默切换） |
| 所有模型都失败 | "抱歉，AI 服务暂时不可用，请稍后再试" |
| Auth 凭证问题（长时间） | "AI 服务配置异常，请检查 API Key" |

### 7. 可观测性

- 每次 failover 记录结构化日志：
  ```
  {"event":"llm_fallback","from":"minimax/M2.7","to":"qwen3-max","reason":"rate_limit","attempt":2}
  ```
- Cooldown 触发时记录告警
- 关键指标：Fallback 触发频率、各模型成功率

## 实现计划

### 步骤一：错误分类与重试逻辑（新增 `llm/retry.go`）

- 定义 `LLMError` 类型及其分类方法
- 实现指数退避重试装饰器
- 实现 Cooldown 管理（内存 + 持久化到 `~/.closeclaw/llm_cooldowns.json`）

### 步骤二：Fallback Chain 实现（`llm/client.go`）

- 在现有 LLM Client 基础上封装 Fallback 逻辑
- 实现 Session 级别的 Auth Profile 绑定
- 支持配置化 fallback models

### 步骤三：配置项接入（`config/schema.go` + `config.yaml`）

- 增加 `llm.*` 配置项
- 支持从环境变量读取（如 `LLM_PRIMARY_MODEL`）

### 步骤四：测试覆盖

- Mock 各类型错误，验证分类正确性
- 测试 Cooldown 退避逻辑
- 测试 Fallback Chain 完整流程

## 扩展性

1. **Provider 级别 Failover**：如果 CloseClaw 未来支持多 Provider，可类似 OpenClaw 增加 `auth.profile` 轮换机制
2. **Per-Intent Fallback**：不同类型任务可用不同模型（如代码任务用 Codex，聊天用 GPT）
3. **用户自定义 Fallback**：高级用户可配置自己的 fallback chain

## 风险与注意事项

1. **幂等性**：部分 LLM 调用（尤其是 tool use）不是幂等的，重试前需确认是否可以安全重试
2. **超时设置**：建议单次调用超时不超过 30s，重试总等待不超过 60s，避免用户等待过长
3. **Cooldown 持久化**：多实例部署时需要共享 cooldown 状态（建议 Redis 或文件锁）
4. **Billing 误判**：部分 Provider 的 429 可能是 Billing 导致的，需要正确区分
