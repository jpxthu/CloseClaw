# audit 模块规格说明书

## 模块概述

审计日志模块负责将权限检查、Agent 启停、配置变更、错误等关键操作记录到持久化 JSONL 文件中（`~/.closeclaw/audit/YYYY-MM-DD.jsonl`），支持日志查询和导出。

所有事件同时发送到 `tracing`（`info`/`error` level），内存缓冲满（500条）时自动刷盘，进程退出时强制刷盘。

---

## 公开接口

### 类型

| 接口名 | 一句话功能 |
|--------|-----------|
| `AuditEventType` | 审计事件类型枚举：PermissionCheck、AgentStart、AgentStop、AgentError、ConfigReload、RuleReload |
| `AuditResult` | 操作结果枚举：Allow、Deny、Error |
| `AuditEvent` | 单条审计事件结构体，含时间戳、类型、详情、结果；含 `new()` 构造方法、`serialize_to_json()` 序列化方法 |
| `AuditEventBuilder` | AuditEvent 的建造者，支持链式构造 |
| `AuditQueryFilter` | 查询过滤器结构体：天数、事件类型（模糊匹配）、Agent名称（模糊匹配）、返回条数上限 |
| `AuditLogger` | 异步审计日志写入器，内置缓冲队列 |

### 构造

| 接口名 | 一句话功能 |
|--------|-----------|
| `AuditLogger::new()` | 创建使用默认路径（`~/.closeclaw/audit`）的 AuditLogger |
| `AuditLogger::with_base_dir(PathBuf)` | 创建使用自定义目录的 AuditLogger（用于测试） |

### 主操作

| 接口名 | 一句话功能 |
|--------|-----------|
| `AuditLogger::log(AuditEvent)` | 异步记录事件；先发往 tracing，再入缓冲队列，队列≥500条时自动刷盘 |
| `AuditLogger::flush()` | 同步将缓冲队列中的所有事件写入当日 JSONL 文件 |
| `AuditLogger::buffer_len()` | 返回当前缓冲队列中的事件数量 |
| `AuditLogger::shutdown()` | 退出时调用，等价于一次 `flush()` |
| `AuditLogger::rotate_if_needed()` | 检测日期变化；若跨天则刷盘并切换到新日期文件 |

### 查询 / 导出

| 接口名 | 一句话功能 |
|--------|-----------|
| `query_audit_events(&AuditQueryFilter) -> Vec<AuditEvent>` | 在指定天数范围内读取并过滤 JSONL 文件，返回按时间倒序的事件列表 |
| `export_audit_events(&AuditQueryFilter, output_path: &str, format: &str) -> usize` | 将查询结果导出为 JSON（pretty）或 JSONL 文件，返回导出条数 |

### 常量

| 接口名 | 值 | 说明 |
|--------|-----|------|
| `MAX_QUERY_DAYS` | 365 | 单次查询最大天数上限，防止 DoS |

---

## 架构 / 结构

### 文件组织

```
src/audit/
├── mod.rs          # 模块声明 + pub use re-exports
├── types.rs        # AuditEventType, AuditResult, AuditEvent, AuditEventBuilder
├── logger.rs       # AuditLogger struct + impl + Default
├── query.rs        # AuditQueryFilter, query_audit_events, export_audit_events
└── tests.rs        # 单元测试
```

### 数据流

1. **记录路径**：`AuditEvent` → `AuditLogger::log()` → 缓冲队列（VecDeque，容量1000）→ 达到500条阈值或显式 `flush()` 时写入 `~/.closeclaw/audit/YYYY-MM-DD.jsonl`
2. **查询路径**：`query_audit_events()` → 读取多日 JSONL 文件 → 反序列化 → 按 filter 过滤 → 限制条数 → 排序返回
3. **导出路径**：调用 `query_audit_events()` 获取结果 → 按指定格式（json/jsonl）写入 output_path

### 跨模块依赖

- `chrono::Local`：获取本地时间戳
- `serde`：JSON 序列化/反序列化
- `tokio::sync::Mutex`：异步缓冲队列的线程安全访问
- `tracing`（`info!` / `error!`）：日志同时写入 tracing 系统
