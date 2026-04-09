# SPEC: Bootstrap 上下文保护层 — Compaction 防护机制

> Issue: [#164](https://github.com/jpxthu/CloseClaw/issues/164)

## 1. 概述

本文档定义 CloseClaw 的 **Bootstrap 上下文保护层**设计。核心目标：

1. 在 OpenClaw 会话触发 compaction 时，确保 agent 的 bootstrap 文件（AGENTS.md、SOUL.md、IDENTITY.md、USER.md）不被摘要扭曲
2. Compaction 完成后，将完整的 bootstrap 内容重新 prepend 到 context 前部
3. 通过 transcript 中的起止标记实现对 compaction 操作的精确控制

## 2. 背景

CloseClaw 中每个 agent 的 bootstrap 文件在会话启动时注入模型上下文。OpenClaw 的 pre-compaction memory flush 机制保护的是**对话产生的记忆**（memory/YYYY-MM-DD.md），但**不保护 bootstrap 文件本身**。

当会话长度触发 compaction 时，包含 bootstrap 文件内容的那段 context 会被摘要，agent 的角色定义、操作规程、人格设定会永久丢失——即使随后重新注入，也只是截断后的版本。

## 3. 现有结构

### 3.1 OpenClaw 框架层

OpenClaw 提供 `/compact` meta command 和 `session_before_compact` / `session_after_compact` hooks。CloseClaw 通过这些 hooks 实现保护逻辑。

### 3.2 CloseClaw Session 模块

现有 `src/session/` 模块包含：
- `persistence.rs` — CheckpointManager（已实现）
- `events.rs` — CheckpointTrigger（已实现）
- `recovery.rs` — SessionRecoveryService（已实现）
- `storage/` — MemoryStorage / RedisStorage（已实现）

### 3.3 Bootstrap 文件

每个 agent workspace 包含：
- `AGENTS.md` — agent 行为规范
- `SOUL.md` — agent 人格设定
- `IDENTITY.md` — agent 身份定义
- `USER.md` — 用户相关信息

这些文件在会话启动时由 OpenClaw 的 `resolveBootstrapFilesForRun` / `buildBootstrapContextFiles` 机制注入。

## 4. 数据结构设计

### 4.1 BootstrapRegion 标记结构

```rust
// src/session/bootstrap.rs

use serde::{Deserialize, Serialize};

/// Bootstrap region markers in the transcript.
/// Placed before and after the bootstrap content to delimit it from regular messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapRegion {
    /// Unique region identifier (session-scoped)
    pub region_id: String,
    /// File path of the bootstrap file
    pub file_path: String,
    /// SHA-256 hash of the original file content (for integrity check)
    pub content_hash: String,
    /// Character count of the original content
    pub char_count: usize,
    /// Whether this is the original injection or a re-injection after compaction
    pub is_reinject: bool,
    /// Original injection timestamp
    pub injected_at: chrono::DateTime<chrono::Utc>,
}

/// Marker text placed before bootstrap content in transcript
pub const BOOTSTRAP_REGION_START: &str = "<bootstrap:";
/// Marker text placed after bootstrap content in transcript
pub const BOOTSTRAP_REGION_END: &str = "</bootstrap>";
```

### 4.2 BootstrapContext 元数据

```rust
/// Bootstrap context metadata stored alongside session state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapContext {
    /// All bootstrap regions currently in the transcript
    pub regions: Vec<BootstrapRegion>,
    /// Whether bootstrap has been re-injected after the last compaction
    pub reinjected_after_last_compact: bool,
    /// Total character count of all bootstrap content
    pub total_char_count: usize,
}

impl Default for BootstrapContext {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
            reinjected_after_last_compact: true, // starts true (no compaction yet)
            total_char_count: 0,
        }
    }
}
```

### 4.3 CompactEvent 扩展

复用现有的 `CheckpointTrigger`，扩展以支持 compaction 事件：

```rust
// 复用 src/session/events.rs 中的 CheckpointTrigger
// 新增 CompactTrigger 变体（在 events.rs 中扩展）
```

## 5. 核心逻辑

### 5.1 会话初始化

1. Agent 启动时，OpenClaw 注入 bootstrap 文件内容到 context
2. **CloseClaw 的 agent process 模块**在注入后执行：
   - 扫描 transcript，找到 bootstrap 内容区域（通过启发式或 OpenClaw hook）
   - 用 `BootstrapRegion` 标记该区域起止
   - 存储 `BootstrapContext` 元数据到 session state

### 5.2 Compaction 拦截（核心）

当收到 `/compact` 命令或 `session_before_compact` hook 触发时：

1. **Before Compaction**:
   - 检查 `BootstrapContext.regions` 是否非空
   - 如果为空：记录警告日志，跳过（无 bootstrap 需要保护）
   - 记录当前所有 `BootstrapRegion` 的 `content_hash`（用于后续完整性校验）

2. **Compaction 执行**:
   - OpenClaw 执行 transcript 摘要
   - **关键**：compaction 操作应跳过 `BOOTSTRAP_REGION_START` 和 `BOOTSTRAP_REGION_END` 标记之间的内容
   - 如果 OpenClaw 不支持跳过：CloseClaw 在 compaction 后检测 bootstrap 内容是否被扭曲

3. **After Compaction**:
   - 检测 bootstrap 内容是否完整（对比 `content_hash`）
   - 如果被扭曲或缺失：执行 reinject
   - 如果完整：标记 `reinjected_after_last_compact = false`
   - 更新 session state 中的 `BootstrapContext`

### 5.3 Reinject 机制

当需要重新注入 bootstrap 内容时：

1. 从 agent workspace 读取原始 bootstrap 文件（AGENTS.md、SOUL.md、IDENTITY.md、USER.md）
2. 计算新的 `content_hash`
3. 在 transcript 前部追加：
   ```
   <bootstrap:file=AGENTS.md,hash=abc123,chars=1234,reinject=true>
   [AGENTS.md 完整内容]
   </bootstrap>
   ```
4. 更新 `BootstrapContext.regions` 中的 `is_reinject=true`
5. 更新 `total_char_count`

### 5.4 Token 成本控制

- 验收标准要求每次 prepend ≤ 60K chars
- 监控 `BootstrapContext.total_char_count`，超过阈值时记录警告
- bootstrap 文件本身一般 < 50K，风险可控

## 6. 实现位置

```
src/
  session/
    bootstrap.rs        # NEW: BootstrapRegion, BootstrapContext, protect/unprotect logic
    events.rs           # EXTEND: Add CompactTrigger variant
    mod.rs              # EXTEND: pub mod bootstrap
  agent/
    process.rs          # EXTEND: Hook into OpenClaw compaction lifecycle
    config.rs           # EXTEND: Bootstrap file path resolution
```

## 7. 测试策略

### 7.1 单元测试

- `test_bootstrap_region_serialization` — 序列化/反序列化
- `test_bootstrap_context_default` — 默认状态
- `test_bootstrap_context_total_char_count` — 字符统计
- `test_content_hash_integrity` — SHA-256 hash 计算与校验
- `test_reinject_appends_new_region` — reinject 添加新 region 而非修改旧 region
- `test_compact_skips_regions` — compaction 跳过标记区域（mock）

### 7.2 集成测试（需要 OpenClaw mock）

- `test_long_session_3_compacts_preserves_agents_rules` — 验收标准 1
- `test_compact前后_bootstrap内容一致` — 验收标准 2
- `test_token_growth_within_60k_limit` — 验收标准 3

## 8. 验收标准

- [ ] 长会话（触发 3 次以上 compaction）后，agent 仍能正确复述 AGENTS.md 中的禁止规则
- [ ] Compaction 前后的 bootstrap 上下文内容一致（允许 head+tail 截断，不可接受摘要扭曲）
- [ ] Token 增长线性可控（每次 prepend 的 bootstrap 内容不超过 60K）

## 9. 依赖

- OpenClaw 的 compaction hook（`session_before_compact` / `session_after_compact`）是否暴露足够接口
- 需要在 CloseClaw 的 agent session 管理层面实现，不修改 OpenClaw 核心
